use std::{env, time::Duration};

use anyhow::{Context, Result, bail};
use api_types::prover::{CircuitKind, JobRequest, JobStatus, JobStatusResponse, SubmitJobResponse};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use reqwest::StatusCode;
use tokio::time::sleep;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROVER_URL_ENV: &str = "DECIDER_PROVER_URL";

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let base_url =
        env::var(PROVER_URL_ENV).with_context(|| format!("{PROVER_URL_ENV} is not set"))?;
    let base_url = base_url.trim_end_matches('/').to_owned();

    let client = reqwest::Client::new();
    let job_id = format!("dummy-root-{}", Utc::now().timestamp_millis());
    let ivc_proof = BASE64_STANDARD.encode(b"dummy-root-proof");
    let request = JobRequest {
        job_id: job_id.clone(),
        circuit: CircuitKind::Root,
        ivc_proof,
    };

    let submit = submit_job(&client, &base_url, &request).await?;
    println!(
        "submitted job {} (status: {})",
        submit.job_id, submit.status
    );

    let final_status = wait_for_completion(&client, &base_url, &job_id).await?;
    match final_status.status {
        JobStatus::Completed => {
            println!("job {} completed", final_status.job_id);
            if let Some(result) = final_status.result {
                println!("proof (base64): {result}");
            }
        }
        JobStatus::Failed => {
            println!("job {} failed", final_status.job_id);
            if let Some(error) = final_status.error {
                println!("error: {error}");
            }
        }
        other => {
            println!(
                "job {} finished with unexpected status {other}",
                final_status.job_id
            );
        }
    }

    Ok(())
}

async fn submit_job(
    client: &reqwest::Client,
    base_url: &str,
    request: &JobRequest,
) -> Result<SubmitJobResponse> {
    let url = format!("{base_url}/jobs");
    let response = client.post(url).json(request).send().await?;
    response
        .error_for_status()
        .context("failed to submit job")?
        .json::<SubmitJobResponse>()
        .await
        .context("invalid submit response")
}

async fn wait_for_completion(
    client: &reqwest::Client,
    base_url: &str,
    job_id: &str,
) -> Result<JobStatusResponse> {
    let url = format!("{base_url}/jobs/{job_id}");
    loop {
        let response = client.get(&url).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            println!("job {job_id} not visible yet, retrying...");
            sleep(POLL_INTERVAL).await;
            continue;
        }

        if response.status() != StatusCode::OK {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("status query failed with {status}: {text}");
        }

        let status = response.json::<JobStatusResponse>().await?;
        println!("job {job_id} status: {}", status.status);
        match status.status {
            JobStatus::Queued | JobStatus::Processing => {}
            _ => return Ok(status),
        }
        sleep(POLL_INTERVAL).await;
    }
}
