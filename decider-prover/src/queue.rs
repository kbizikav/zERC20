use deadpool_redis::{
    Config as RedisConfig, Pool, Runtime,
    redis::{AsyncCommands, Value},
};

use api_types::prover::{CircuitKind, JobInfoResponse, JobRequest, JobStatus, JobStatusResponse};
use serde::{Deserialize, Serialize};

use crate::errors::ProverError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobState {
    pub job_id: String,
    pub circuit: CircuitKind,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<JobState> for JobStatusResponse {
    fn from(state: JobState) -> Self {
        Self {
            job_id: state.job_id,
            circuit: state.circuit,
            status: state.status,
            result: state.result,
            error: state.error,
        }
    }
}

impl From<&JobState> for JobInfoResponse {
    fn from(state: &JobState) -> Self {
        Self {
            job_id: state.job_id.clone(),
            circuit: state.circuit.clone(),
            status: state.status.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueJobResult {
    Enqueued(JobState),
    AlreadyExists(JobState),
}

pub fn create_pool(redis_url: &str) -> Result<Pool, ProverError> {
    let cfg = RedisConfig::from_url(redis_url);
    cfg.create_pool(Some(Runtime::Tokio1))
        .map_err(ProverError::from)
}

pub async fn get_job(
    pool: &Pool,
    job_prefix: &str,
    job_id: &str,
) -> Result<Option<JobState>, ProverError> {
    let mut conn = pool.get().await?;
    let key = job_key(job_prefix, job_id);
    let value: Option<String> = conn.get(&key).await?;
    drop(conn);
    match value {
        Some(data) => Ok(Some(serde_json::from_str(&data)?)),
        None => Ok(None),
    }
}

pub async fn save_job(
    pool: &Pool,
    job_prefix: &str,
    job: &JobState,
    ttl_seconds: u64,
) -> Result<(), ProverError> {
    let mut conn = pool.get().await?;
    let key = job_key(job_prefix, &job.job_id);
    let payload = serde_json::to_string(job)?;
    let _: () = conn.set_ex(key, payload, ttl_seconds).await?;
    Ok(())
}

pub async fn enqueue_job(
    pool: &Pool,
    queue_key: &str,
    job_prefix: &str,
    request: &JobRequest,
    ttl_seconds: u64,
) -> Result<EnqueueJobResult, ProverError> {
    let state = JobState {
        job_id: request.job_id.clone(),
        circuit: request.circuit.clone(),
        status: JobStatus::Queued,
        result: None,
        error: None,
    };
    enqueue_job_atomically(pool, queue_key, job_prefix, request, &state, ttl_seconds).await
}

pub async fn wait_for_job(pool: &Pool, queue_key: &str) -> Result<JobRequest, ProverError> {
    let mut conn = pool.get().await?;
    let response: Option<(String, String)> = deadpool_redis::redis::cmd("BLPOP")
        .arg(queue_key)
        .arg(0)
        .query_async(&mut *conn)
        .await?;
    drop(conn);
    let (_, payload) = response.ok_or_else(|| {
        ProverError::InvalidInput("unexpected empty response from Redis queue".to_owned())
    })?;
    let job: JobRequest = serde_json::from_str(&payload)?;
    Ok(job)
}

pub async fn update_status(
    pool: &Pool,
    job_prefix: &str,
    job_id: &str,
    status: JobStatus,
    result: Option<String>,
    error: Option<String>,
    ttl_seconds: u64,
) -> Result<JobState, ProverError> {
    let mut job = get_job(pool, job_prefix, job_id)
        .await?
        .ok_or_else(|| ProverError::InvalidInput(format!("job {job_id} not found")))?;
    job.status = status;
    job.result = result;
    job.error = error;
    save_job(pool, job_prefix, &job, ttl_seconds).await?;
    Ok(job)
}

fn job_key(prefix: &str, job_id: &str) -> String {
    format!("{prefix}:{job_id}")
}

const ENQUEUE_JOB_LUA: &str = r#"
local job_key = KEYS[1]
local queue_key = KEYS[2]
local job_state = ARGV[1]
local queue_payload = ARGV[2]
local ttl_seconds = ARGV[3]

if redis.call("EXISTS", job_key) == 1 then
    redis.call("EXPIRE", job_key, ttl_seconds)
    return {0, redis.call("GET", job_key)}
end

redis.call("SET", job_key, job_state, "EX", ttl_seconds)
redis.call("RPUSH", queue_key, queue_payload)
return {1, job_state}
"#;

async fn enqueue_job_atomically(
    pool: &Pool,
    queue_key: &str,
    job_prefix: &str,
    request: &JobRequest,
    state: &JobState,
    ttl_seconds: u64,
) -> Result<EnqueueJobResult, ProverError> {
    let mut conn = pool.get().await?;
    let key = job_key(job_prefix, &state.job_id);
    let job_state_json = serde_json::to_string(state)?;
    let queue_payload = serde_json::to_string(request)?;
    let ttl_arg = ttl_seconds.to_string();

    let response: Value = deadpool_redis::redis::cmd("EVAL")
        .arg(ENQUEUE_JOB_LUA)
        .arg(2)
        .arg(&key)
        .arg(queue_key)
        .arg(job_state_json)
        .arg(queue_payload)
        .arg(ttl_arg)
        .query_async(&mut *conn)
        .await?;

    parse_enqueue_response(response)
}

fn parse_enqueue_response(value: Value) -> Result<EnqueueJobResult, ProverError> {
    let Value::Bulk(items) = value else {
        return Err(ProverError::InvalidInput(format!(
            "unexpected enqueue response {value:?}"
        )));
    };

    if items.len() != 2 {
        return Err(ProverError::InvalidInput(format!(
            "unexpected enqueue response length {}",
            items.len()
        )));
    }

    let inserted = parse_enqueue_flag(&items[0])?;
    let state = parse_job_state(&items[1])?;

    Ok(if inserted {
        EnqueueJobResult::Enqueued(state)
    } else {
        EnqueueJobResult::AlreadyExists(state)
    })
}

fn parse_enqueue_flag(value: &Value) -> Result<bool, ProverError> {
    match value {
        Value::Int(num) => Ok(*num == 1),
        Value::Status(s) => Ok(s == "1"),
        Value::Data(bytes) => {
            let parsed = String::from_utf8(bytes.clone())
                .map_err(|_| ProverError::InvalidInput("invalid enqueue flag".to_owned()))?;
            Ok(parsed == "1")
        }
        other => Err(ProverError::InvalidInput(format!(
            "unexpected enqueue flag type {other:?}"
        ))),
    }
}

fn parse_job_state(value: &Value) -> Result<JobState, ProverError> {
    let json = match value {
        Value::Data(bytes) => String::from_utf8(bytes.clone())
            .map_err(|_| ProverError::InvalidInput("invalid job payload".to_owned()))?,
        Value::Status(s) => s.clone(),
        other => {
            return Err(ProverError::InvalidInput(format!(
                "unexpected job payload type {other:?}"
            )));
        }
    };
    Ok(serde_json::from_str(&json)?)
}
