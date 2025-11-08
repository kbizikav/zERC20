use std::time::{Duration, Instant};

use api_types::prover::{CircuitKind, JobRequest, JobStatus, JobStatusResponse};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use reqwest::{Client, Url};
use thiserror::Error;
use tokio::time::sleep;

const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait DeciderClient: Send + Sync {
    async fn produce_decider_proof(
        &self,
        circuit: CircuitKind,
        ivc_proof: &[u8],
    ) -> DeciderResult<Vec<u8>>;
}

pub struct HttpDeciderClient {
    client: Client,
    base_url: Url,
    poll_interval: Duration,
    timeout: Duration,
}

impl HttpDeciderClient {
    pub fn new(base_url: Url, poll_interval: Duration, timeout: Duration) -> DeciderResult<Self> {
        let mut normalized = base_url.clone();
        if !normalized.path().ends_with('/') {
            let mut path = normalized.path().trim_end_matches('/').to_owned();
            path.push('/');
            normalized.set_path(&path);
        }

        let client = Client::builder()
            .build()
            .map_err(DeciderError::ClientBuild)?;

        Ok(Self {
            client,
            base_url: normalized,
            poll_interval,
            timeout,
        })
    }

    pub fn with_defaults(base_url: Url, timeout: Duration) -> DeciderResult<Self> {
        Self::new(
            base_url,
            Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
            timeout,
        )
    }
}

#[derive(Debug, Error)]
pub enum DeciderError {
    #[error("failed to build HTTP client for prover")]
    ClientBuild(#[source] reqwest::Error),
    #[error("invalid prover base url while constructing {path}")]
    InvalidEndpoint {
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to submit {circuit} prover job")]
    SubmitJob {
        circuit: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{circuit} prover job submission returned error")]
    SubmitJobStatus {
        circuit: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to query prover job status for {circuit}")]
    StatusRequest {
        circuit: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{circuit} prover job status endpoint returned error")]
    StatusRequestStatus {
        circuit: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to decode {circuit} prover job status response")]
    StatusDecode {
        circuit: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("prover job {job_id} completed without result")]
    MissingResult { job_id: String },
    #[error("invalid prover result payload")]
    InvalidResultPayload(#[source] base64::DecodeError),
    #[error("base64 payload is empty")]
    EmptyPayload,
    #[error("{circuit} prover job {job_id} failed: {error_msg}")]
    JobFailed {
        circuit: String,
        job_id: String,
        error_msg: String,
    },
    #[error("timed out waiting for prover job {job_id} after {timeout:?}")]
    Timeout { job_id: String, timeout: Duration },
}

pub type DeciderResult<T> = Result<T, DeciderError>;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl DeciderClient for HttpDeciderClient {
    async fn produce_decider_proof(
        &self,
        circuit: CircuitKind,
        ivc_proof: &[u8],
    ) -> DeciderResult<Vec<u8>> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let circuit_name = circuit.to_string();

        let submit_url =
            self.base_url
                .join("jobs")
                .map_err(|source| DeciderError::InvalidEndpoint {
                    path: "jobs".to_string(),
                    source,
                })?;

        let ivc_proof_b64 = BASE64_STANDARD.encode(ivc_proof);

        let request = JobRequest {
            job_id: job_id.clone(),
            circuit: circuit.clone(),
            ivc_proof: ivc_proof_b64,
        };

        self.client
            .post(submit_url)
            .json(&request)
            .send()
            .await
            .map_err(|source| DeciderError::SubmitJob {
                circuit: circuit_name.clone(),
                source,
            })?
            .error_for_status()
            .map_err(|source| DeciderError::SubmitJobStatus {
                circuit: circuit_name.clone(),
                source,
            })?;

        let status_url = self
            .base_url
            .join(&format!("jobs/{job_id}"))
            .map_err(|source| DeciderError::InvalidEndpoint {
                path: format!("jobs/{job_id}"),
                source,
            })?;

        let started = Instant::now();
        loop {
            if started.elapsed() > self.timeout {
                return Err(DeciderError::Timeout {
                    job_id: job_id.clone(),
                    timeout: self.timeout,
                });
            }

            let response = self
                .client
                .get(status_url.clone())
                .send()
                .await
                .map_err(|source| DeciderError::StatusRequest {
                    circuit: circuit_name.clone(),
                    source,
                })?
                .error_for_status()
                .map_err(|source| DeciderError::StatusRequestStatus {
                    circuit: circuit_name.clone(),
                    source,
                })?;

            let status: JobStatusResponse =
                response
                    .json()
                    .await
                    .map_err(|source| DeciderError::StatusDecode {
                        circuit: circuit_name.clone(),
                        source,
                    })?;

            match status.status {
                JobStatus::Queued | JobStatus::Processing => {
                    sleep(self.poll_interval).await;
                }
                JobStatus::Completed => {
                    let result = status.result.ok_or_else(|| DeciderError::MissingResult {
                        job_id: job_id.clone(),
                    })?;
                    let proof = decode_base64_payload(&result)?;
                    return Ok(proof);
                }
                JobStatus::Failed => {
                    let err = status.error.unwrap_or_else(|| "unknown error".to_string());
                    return Err(DeciderError::JobFailed {
                        circuit: circuit_name.clone(),
                        job_id: job_id.clone(),
                        error_msg: err,
                    });
                }
            }
        }
    }
}

pub fn decode_base64_payload(value: &str) -> DeciderResult<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DeciderError::EmptyPayload);
    }
    BASE64_STANDARD
        .decode(trimmed)
        .map_err(DeciderError::InvalidResultPayload)
}

#[cfg(test)]
mod tests {
    use super::decode_base64_payload;
    use base64::Engine;

    #[test]
    fn decode_base64_payload_decodes() {
        let data = vec![0xde, 0xad, 0xbe, 0xef];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let bytes = decode_base64_payload(&encoded).expect("decode base64");
        assert_eq!(bytes, data);
    }
}
