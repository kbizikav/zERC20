use std::{sync::Arc, time::Instant};

use actix_cors::Cors;
use actix_web::{
    App, HttpResponse, HttpServer, Responder, error, get,
    middleware::Logger,
    post,
    web::{Data, Json, JsonConfig, Path},
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use deadpool_redis::Pool;
use log::{error as log_error, info};

use decider_prover::{
    CircuitKind, JobRequest, JobStatusResponse, ProverEngine, ProverError, SubmitJobResponse,
    config::{AppConfig, load_config},
    queue::{self, EnqueueJobResult},
    worker,
};

#[derive(Clone)]
struct AppState {
    pool: Pool,
    queue_key: String,
    job_prefix: String,
    job_ttl_seconds: u64,
    enable_withdraw_local: bool,
}

#[post("/jobs")]
async fn submit_job(
    state: Data<AppState>,
    payload: Json<JobRequest>,
) -> actix_web::Result<impl Responder> {
    let request = payload.into_inner();

    validate_job_request(&request, state.enable_withdraw_local)?;

    match queue::enqueue_job(
        &state.pool,
        &state.queue_key,
        &state.job_prefix,
        &request,
        state.job_ttl_seconds,
    )
    .await
    {
        Ok(EnqueueJobResult::Enqueued(job_state)) => {
            let response = SubmitJobResponse {
                job_id: job_state.job_id,
                status: job_state.status,
                message: "job queued".to_owned(),
            };
            Ok(HttpResponse::Accepted().json(response))
        }
        Ok(EnqueueJobResult::AlreadyExists(existing)) => {
            let job_id = existing.job_id.clone();
            let status = existing.status.clone();
            let message = format!("job {job_id} already exists with status {}", status);
            let response = SubmitJobResponse {
                job_id,
                status,
                message,
            };
            Ok(HttpResponse::Ok().json(response))
        }
        Err(err) => Err(map_internal_error(err)),
    }
}

#[get("/jobs/{job_id}")]
async fn get_job_status(
    state: Data<AppState>,
    job_id: Path<String>,
) -> actix_web::Result<impl Responder> {
    let job_id = job_id.into_inner();
    match queue::get_job(&state.pool, &state.job_prefix, &job_id).await {
        Ok(Some(state)) => Ok(HttpResponse::Ok().json(JobStatusResponse::from(state))),
        Ok(None) => Err(error::ErrorNotFound(format!("job {job_id} not found"))),
        Err(err) => Err(map_internal_error(err)),
    }
}

#[get("/healthz")]
async fn health() -> impl Responder {
    HttpResponse::Ok().finish()
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    env_logger::init();

    let config = load_config()?;
    info!(
        "loading decider parameters from {}",
        config.artifacts_dir.display()
    );
    let engine = Arc::new(load_prover_engine(&config)?);
    let pool = queue::create_pool(&config.redis_url)?;

    worker::spawn_worker(
        pool.clone(),
        Arc::clone(&engine),
        config.queue_key.clone(),
        config.job_key_prefix.clone(),
        config.job_ttl_seconds,
    );

    let app_state = Data::new(AppState {
        pool,
        queue_key: config.queue_key.clone(),
        job_prefix: config.job_key_prefix.clone(),
        job_ttl_seconds: config.job_ttl_seconds,
        enable_withdraw_local: config.enable_withdraw_local,
    });
    let json_limit = config.json_body_limit_bytes;

    info!("starting server on {}", config.listen_addr);
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .wrap(Logger::default())
            .app_data(app_state.clone())
            .app_data(JsonConfig::default().limit(json_limit))
            .service(submit_job)
            .service(get_job_status)
            .service(health)
    })
    .bind(config.listen_addr.clone())?
    .run()
    .await?;

    Ok(())
}

fn load_prover_engine(config: &AppConfig) -> Result<ProverEngine, ProverError> {
    let start = Instant::now();
    let engine = ProverEngine::load(&config.artifacts_dir, config.enable_withdraw_local)?;
    info!(
        "loaded decider parameters from {} in {:.2?}",
        config.artifacts_dir.display(),
        start.elapsed()
    );
    Ok(engine)
}

fn validate_job_request(
    request: &JobRequest,
    enable_withdraw_local: bool,
) -> actix_web::Result<()> {
    if request.job_id.trim().is_empty() {
        return Err(error::ErrorBadRequest("job_id must not be empty"));
    }
    match request.circuit {
        CircuitKind::Root | CircuitKind::WithdrawGlobal => {}
        CircuitKind::WithdrawLocal if enable_withdraw_local => {}
        CircuitKind::WithdrawLocal => {
            return Err(error::ErrorBadRequest(
                "withdraw_local circuit is disabled in this prover",
            ));
        }
    }
    ensure_base64_payload(&request.ivc_proof)?;
    Ok(())
}

fn ensure_base64_payload(value: &str) -> actix_web::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(error::ErrorBadRequest("ivc_proof payload is empty"));
    }
    BASE64_STANDARD
        .decode(trimmed)
        .map(|_| ())
        .map_err(|_| error::ErrorBadRequest("ivc_proof must be valid base64"))?;
    Ok(())
}

fn map_internal_error(err: ProverError) -> actix_web::Error {
    match err {
        ProverError::InvalidInput(message) => error::ErrorBadRequest(message),
        ProverError::HexDecode(_) => error::ErrorBadRequest("invalid hex input"),
        other => {
            log_error!("internal error: {other}");
            error::ErrorInternalServerError("internal server error")
        }
    }
}
