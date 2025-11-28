use std::{sync::Arc, time::Duration};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use log::{error, info};
use tokio::time::sleep;

use deadpool_redis::Pool;

use api_types::prover::JobStatus;

use crate::{circuits::ProverEngine, errors::ProverError, queue};

pub fn spawn_worker(
    pool: Pool,
    engine: Arc<ProverEngine>,
    queue_key: String,
    job_prefix: String,
    job_ttl_seconds: u64,
) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = process_once(
                &pool,
                engine.as_ref(),
                &queue_key,
                &job_prefix,
                job_ttl_seconds,
            )
            .await
            {
                error!("worker error: {err}");
                sleep(Duration::from_secs(1)).await;
            }
        }
    });
}

async fn process_once(
    pool: &Pool,
    engine: &ProverEngine,
    queue_key: &str,
    job_prefix: &str,
    job_ttl_seconds: u64,
) -> Result<(), ProverError> {
    let job = queue::wait_for_job(pool, queue_key).await?;
    let job_id = job.job_id.clone();
    let circuit = job.circuit.clone();

    info!("job {job_id} ({circuit}) started");

    queue::update_status(
        pool,
        job_prefix,
        &job_id,
        JobStatus::Processing,
        None,
        None,
        job_ttl_seconds,
    )
    .await?;

    let proof_bytes = decode_base64(&job.ivc_proof)?;
    match engine.generate_decider_proof(circuit.clone(), &proof_bytes) {
        Ok(proof) => {
            let result_b64 = BASE64_STANDARD.encode(&proof);
            queue::update_status(
                pool,
                job_prefix,
                &job_id,
                JobStatus::Completed,
                Some(result_b64),
                None,
                job_ttl_seconds,
            )
            .await?;
            info!("job {job_id} ({circuit}) completed");
        }
        Err(err) => {
            error!("job {job_id} ({circuit}) failed: {err}");
            queue::update_status(
                pool,
                job_prefix,
                &job_id,
                JobStatus::Failed,
                None,
                Some(truncate_error(err.to_string())),
                job_ttl_seconds,
            )
            .await?;
        }
    }

    Ok(())
}

fn decode_base64(input: &str) -> Result<Vec<u8>, ProverError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ProverError::InvalidInput(
            "ivc_proof payload is empty".to_owned(),
        ));
    }
    BASE64_STANDARD
        .decode(trimmed)
        .map_err(|_| ProverError::InvalidInput("ivc_proof must be valid base64".to_owned()))
}

fn truncate_error(message: String) -> String {
    const MAX_ERROR_LEN: usize = 512;
    if message.len() <= MAX_ERROR_LEN {
        message
    } else {
        format!("{}...", &message[..MAX_ERROR_LEN])
    }
}
