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
use log::{error as log_error, info};

use decider_prover::{
    JobRequest, JobStatusResponse, ProverEngine, ProverError, SubmitJobResponse,
    config::{AppConfig, CircuitEnablement, load_config},
    queue::{EnqueueJobResult, QueueClient},
    worker,
};

#[derive(Clone)]
struct AppState {
    queue: QueueClient,
    job_ttl_seconds: u64,
    enabled_circuits: CircuitEnablement,
}

#[post("/jobs")]
async fn submit_job(
    state: Data<AppState>,
    payload: Json<JobRequest>,
) -> actix_web::Result<impl Responder> {
    let request = payload.into_inner();

    validate_job_request(&request, &state.enabled_circuits)?;

    match state
        .queue
        .enqueue_job(&request, state.job_ttl_seconds)
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
    match state.queue.get_job(&job_id).await {
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
    let queue = QueueClient::connect(
        &config.database_url,
        &config.queue_name,
        &config.job_table,
        config.visibility_timeout_seconds,
        config.visibility_extension_seconds,
    )
    .await?;

    for _ in 0..config.worker_count {
        worker::spawn_worker(queue.clone(), Arc::clone(&engine), config.job_ttl_seconds);
    }
    info!("spawned {} worker(s)", config.worker_count);

    let app_state = Data::new(AppState {
        queue,
        job_ttl_seconds: config.job_ttl_seconds,
        enabled_circuits: config.enabled_circuits.clone(),
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
    let engine = ProverEngine::load(&config.artifacts_dir, &config.enabled_circuits)?;
    info!(
        "loaded decider parameters from {} in {:.2?}",
        config.artifacts_dir.display(),
        start.elapsed()
    );
    Ok(engine)
}

fn validate_job_request(
    request: &JobRequest,
    enabled_circuits: &CircuitEnablement,
) -> actix_web::Result<()> {
    if request.job_id.trim().is_empty() {
        return Err(error::ErrorBadRequest("job_id must not be empty"));
    }
    if !enabled_circuits.contains(&request.circuit) {
        return Err(error::ErrorBadRequest(format!(
            "{} circuit is disabled in this prover",
            request.circuit
        )));
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
