use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use log::{error, info, warn};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{Duration, sleep},
};

use api_types::prover::JobStatus;

use crate::{circuits::ProverEngine, errors::ProverError, queue::QueueClient};

pub fn spawn_worker(queue: QueueClient, engine: Arc<ProverEngine>, job_ttl_seconds: u64) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = process_once(&queue, Arc::clone(&engine), job_ttl_seconds).await {
                error!("worker error: {err}");
                sleep(Duration::from_secs(1)).await;
            }
        }
    });
}

async fn process_once(
    queue: &QueueClient,
    engine: Arc<ProverEngine>,
    job_ttl_seconds: u64,
) -> Result<(), ProverError> {
    let queued_job = queue.wait_for_job().await?;
    let job = queued_job.job;
    let message_id = queued_job.message_id;
    let job_id = job.job_id.clone();
    let circuit = job.circuit.clone();

    info!("job {job_id} ({circuit}) started");

    queue
        .update_status(&job_id, JobStatus::Processing, None, None, job_ttl_seconds)
        .await?;

    let proof_bytes = decode_base64(&job.ivc_proof)?;
    let guard = VisibilityGuard::spawn(
        queue.clone(),
        message_id,
        queue.visibility_extension_interval(),
    );
    let engine_for_task = Arc::clone(&engine);
    let circuit_for_task = circuit.clone();
    let proof_task = tokio::task::spawn_blocking(move || {
        engine_for_task.generate_decider_proof(circuit_for_task, &proof_bytes)
    });
    let proof_result = proof_task
        .await
        .map_err(|err| ProverError::InvalidInput(format!("worker execution error: {err}")))?;
    guard.shutdown().await;

    match proof_result {
        Ok(proof) => {
            let result_b64 = BASE64_STANDARD.encode(&proof);
            queue
                .update_status(
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
            queue
                .update_status(
                    &job_id,
                    JobStatus::Failed,
                    None,
                    Some(truncate_error(err.to_string())),
                    job_ttl_seconds,
                )
                .await?;
        }
    }

    queue.delete_message(message_id).await?;
    queue.clear_message_binding(&job_id).await?;

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

struct VisibilityGuard {
    stop_tx: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

impl VisibilityGuard {
    fn spawn(queue: QueueClient, message_id: i64, interval: Duration) -> Self {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = sleep(interval) => {
                        if let Err(err) = queue.extend_visibility(message_id).await {
                            warn!("failed to extend visibility for message {message_id}: {err}");
                            break;
                        }
                    }
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
            handle,
        }
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.handle.await;
    }
}
