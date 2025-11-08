use std::time::Duration;

use api_types::prover::{CircuitKind, JobInfoResponse, JobRequest, JobStatus, JobStatusResponse};
use chrono::{Duration as ChronoDuration, Utc};
use pgmq::{
    Message, PGMQueue,
    types::{PGMQ_SCHEMA, QUEUE_PREFIX},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use tokio::time::sleep;

use crate::errors::ProverError;

const READ_POLL_INTERVAL_MS: u64 = 250;
const DEFAULT_POOL_SIZE: u32 = 16;

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

impl JobState {
    fn is_terminal(&self) -> bool {
        matches!(self.status, JobStatus::Completed | JobStatus::Failed)
    }

    fn reset_for_retry(&mut self) {
        self.status = JobStatus::Queued;
        self.result = None;
        self.error = None;
    }
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

#[derive(Clone)]
pub struct QueueClient {
    pool: Pool<Postgres>,
    pgmq: PGMQueue,
    queue_name: String,
    queue_table: QueueTableName,
    job_table: JobTableName,
    visibility_timeout_secs: i32,
    visibility_extension_interval: Duration,
}

#[derive(Debug)]
pub struct QueuedJob {
    pub message_id: i64,
    pub job: JobRequest,
}

#[derive(Debug)]
struct JobRecord {
    state: JobState,
    message_id: Option<i64>,
}

impl JobRecord {
    fn into_state(self) -> JobState {
        self.state
    }
}

enum MessageUpdate {
    Leave,
    Set(Option<i64>),
}

impl QueueClient {
    pub async fn connect(
        database_url: &str,
        queue_name: &str,
        job_table: &str,
        visibility_timeout_seconds: i32,
        visibility_extension_seconds: i32,
    ) -> Result<Self, ProverError> {
        let queue_table = QueueTableName::new(queue_name)?;
        let job_table = JobTableName::new(job_table)?;
        let pool = PgPoolOptions::new()
            .max_connections(DEFAULT_POOL_SIZE)
            .connect(database_url)
            .await?;
        let pgmq = PGMQueue::new_with_pool(pool.clone()).await;
        pgmq.create(queue_name).await?;
        create_job_table(&pool, &job_table).await?;

        Ok(Self {
            pool,
            pgmq,
            queue_name: queue_name.to_owned(),
            queue_table,
            job_table,
            visibility_timeout_secs: visibility_timeout_seconds,
            visibility_extension_interval: Duration::from_secs(visibility_extension_seconds as u64),
        })
    }

    pub async fn enqueue_job(
        &self,
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

        loop {
            let mut tx = self.pool.begin().await?;
            purge_expired_job(tx.as_mut(), &self.job_table, &state.job_id).await?;

            if insert_job_if_absent(&mut tx, &self.job_table, &state, ttl_seconds).await? {
                let msg_id = enqueue_message(&mut tx, &self.queue_table, request).await?;
                set_message_binding(tx.as_mut(), &self.job_table, &state.job_id, Some(msg_id))
                    .await?;
                tx.commit().await?;
                return Ok(EnqueueJobResult::Enqueued(state));
            }

            match fetch_job_record(tx.as_mut(), &self.job_table, &state.job_id, true).await? {
                Some(mut record) => {
                    if record.state.is_terminal() {
                        refresh_job_ttl(tx.as_mut(), &self.job_table, &state.job_id, ttl_seconds)
                            .await?;
                        tx.commit().await?;
                        return Ok(EnqueueJobResult::AlreadyExists(record.into_state()));
                    }

                    let mut needs_requeue = record.message_id.is_none();
                    if let Some(message_id) = record.message_id {
                        needs_requeue =
                            !message_exists(tx.as_mut(), &self.queue_table, message_id).await?;
                    }

                    if needs_requeue {
                        record.state.reset_for_retry();
                        let msg_id = enqueue_message(&mut tx, &self.queue_table, request).await?;
                        save_job(
                            tx.as_mut(),
                            &self.job_table,
                            &record.state,
                            ttl_seconds,
                            MessageUpdate::Set(Some(msg_id)),
                        )
                        .await?;
                        tx.commit().await?;
                        return Ok(EnqueueJobResult::Enqueued(record.state));
                    }

                    refresh_job_ttl(tx.as_mut(), &self.job_table, &state.job_id, ttl_seconds)
                        .await?;
                    tx.commit().await?;
                    return Ok(EnqueueJobResult::AlreadyExists(record.into_state()));
                }
                None => {
                    tx.rollback().await?;
                }
            }
        }
    }

    pub async fn get_job(&self, job_id: &str) -> Result<Option<JobState>, ProverError> {
        Ok(fetch_job_record(&self.pool, &self.job_table, job_id, false)
            .await?
            .map(JobRecord::into_state))
    }

    pub async fn wait_for_job(&self) -> Result<QueuedJob, ProverError> {
        loop {
            match self
                .pgmq
                .read::<JobRequest>(&self.queue_name, Some(self.visibility_timeout_secs))
                .await?
            {
                Some(Message {
                    msg_id, message, ..
                }) => {
                    return Ok(QueuedJob {
                        message_id: msg_id,
                        job: message,
                    });
                }
                None => sleep(Duration::from_millis(READ_POLL_INTERVAL_MS)).await,
            }
        }
    }

    pub async fn update_status(
        &self,
        job_id: &str,
        status: JobStatus,
        result: Option<String>,
        error: Option<String>,
        ttl_seconds: u64,
    ) -> Result<JobState, ProverError> {
        let mut job = self
            .get_job(job_id)
            .await?
            .ok_or_else(|| ProverError::InvalidInput(format!("job {job_id} not found")))?;
        job.status = status;
        job.result = result;
        job.error = error;
        save_job(
            &self.pool,
            &self.job_table,
            &job,
            ttl_seconds,
            MessageUpdate::Leave,
        )
        .await?;
        Ok(job)
    }

    pub async fn delete_message(&self, message_id: i64) -> Result<(), ProverError> {
        self.pgmq.delete(&self.queue_name, message_id).await?;
        Ok(())
    }

    pub async fn clear_message_binding(&self, job_id: &str) -> Result<(), ProverError> {
        set_message_binding(&self.pool, &self.job_table, job_id, None).await
    }

    pub async fn extend_visibility(&self, message_id: i64) -> Result<(), ProverError> {
        let new_vt = Utc::now() + ChronoDuration::seconds(i64::from(self.visibility_timeout_secs));
        self.pgmq
            .set_vt::<JobRequest>(&self.queue_name, message_id, new_vt)
            .await?;
        Ok(())
    }

    pub fn visibility_extension_interval(&self) -> Duration {
        self.visibility_extension_interval
    }
}

#[derive(Clone)]
struct QueueTableName {
    qualified: String,
}

impl QueueTableName {
    fn new(name: &str) -> Result<Self, ProverError> {
        ensure_identifier(name, "queue_name")?;
        let table = format!("{}_{}", QUEUE_PREFIX, name);
        let qualified = format!("{}.{}", quote_ident(PGMQ_SCHEMA), quote_ident(&table));
        Ok(Self { qualified })
    }

    fn qualified(&self) -> &str {
        &self.qualified
    }
}

#[derive(Clone)]
struct JobTableName {
    qualified: String,
    index_name: String,
}

impl JobTableName {
    fn new(input: &str) -> Result<Self, ProverError> {
        let (schema, name) = match input.split_once('.') {
            Some((schema, name)) => {
                ensure_identifier(schema, "job_table schema")?;
                ensure_identifier(name, "job_table name")?;
                (Some(schema.to_owned()), name.to_owned())
            }
            None => {
                ensure_identifier(input, "job_table name")?;
                (None, input.to_owned())
            }
        };

        let qualified = match &schema {
            Some(schema) => format!("{}.{}", quote_ident(schema), quote_ident(&name)),
            None => quote_ident(&name),
        };
        let safe_prefix = schema
            .as_deref()
            .map(|s| s.replace('.', "_"))
            .unwrap_or_else(|| "public".to_owned());
        let safe_name = format!("{}_{}", safe_prefix, name);
        let index_name = format!("{}_expires_idx", safe_name);
        Ok(Self {
            qualified,
            index_name,
        })
    }

    fn qualified(&self) -> &str {
        &self.qualified
    }

    fn index_name(&self) -> &str {
        &self.index_name
    }
}

fn ensure_identifier(value: &str, label: &str) -> Result<(), ProverError> {
    let valid = !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(ProverError::Config(format!(
            "{label} must contain only alphanumeric characters or underscores"
        )))
    }
}

fn quote_ident(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        if ch == '"' {
            quoted.push('"');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

async fn create_job_table(pool: &Pool<Postgres>, table: &JobTableName) -> Result<(), ProverError> {
    let create_sql = format!(
        "
        CREATE TABLE IF NOT EXISTS {} (
            job_id TEXT PRIMARY KEY,
            state JSONB NOT NULL,
            message_id BIGINT,
            expires_at TIMESTAMPTZ NOT NULL
        );
        ",
        table.qualified()
    );
    sqlx::query(&create_sql).execute(pool).await?;

    let index_sql = format!(
        "
        CREATE INDEX IF NOT EXISTS {} ON {} (expires_at);
        ",
        quote_ident(table.index_name()),
        table.qualified()
    );
    sqlx::query(&index_sql).execute(pool).await?;
    Ok(())
}

async fn enqueue_message<'a>(
    tx: &mut Transaction<'a, Postgres>,
    table: &QueueTableName,
    request: &JobRequest,
) -> Result<i64, ProverError> {
    let sql = format!(
        "
        INSERT INTO {} (vt, message)
        VALUES (now(), $1::jsonb)
        RETURNING msg_id;
        ",
        table.qualified()
    );
    let payload = serde_json::to_value(request)?;
    let row = sqlx::query(&sql)
        .bind(payload)
        .fetch_one(tx.as_mut())
        .await?;
    row.try_get("msg_id")
        .map_err(|err| ProverError::InvalidInput(format!("missing msg_id: {err}")))
}

async fn insert_job_if_absent<'a>(
    tx: &mut Transaction<'a, Postgres>,
    table: &JobTableName,
    state: &JobState,
    ttl_seconds: u64,
) -> Result<bool, ProverError> {
    let sql = format!(
        "
        INSERT INTO {} (job_id, state, message_id, expires_at)
        VALUES ($1, $2::jsonb, NULL, now() + ($3::bigint * INTERVAL '1 second'))
        ON CONFLICT DO NOTHING;
        ",
        table.qualified()
    );
    let ttl = ttl_to_i64(ttl_seconds)?;
    let payload = serde_json::to_value(state)?;
    let result = sqlx::query(&sql)
        .bind(&state.job_id)
        .bind(payload)
        .bind(ttl)
        .execute(tx.as_mut())
        .await?;
    Ok(result.rows_affected() == 1)
}

async fn refresh_job_ttl<'a, E>(
    executor: E,
    table: &JobTableName,
    job_id: &str,
    ttl_seconds: u64,
) -> Result<(), ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let sql = format!(
        "
        UPDATE {}
        SET expires_at = now() + ($2::bigint * INTERVAL '1 second')
        WHERE job_id = $1;
        ",
        table.qualified()
    );
    let ttl = ttl_to_i64(ttl_seconds)?;
    sqlx::query(&sql)
        .bind(job_id)
        .bind(ttl)
        .execute(executor)
        .await?;
    Ok(())
}

async fn purge_expired_job<'a, E>(
    executor: E,
    table: &JobTableName,
    job_id: &str,
) -> Result<(), ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let sql = format!(
        "
        DELETE FROM {}
        WHERE job_id = $1
          AND expires_at <= now();
        ",
        table.qualified()
    );
    sqlx::query(&sql).bind(job_id).execute(executor).await?;
    Ok(())
}

async fn fetch_job_record<'a, E>(
    executor: E,
    table: &JobTableName,
    job_id: &str,
    for_update: bool,
) -> Result<Option<JobRecord>, ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let mut sql = format!(
        "
        SELECT state, message_id
        FROM {}
        WHERE job_id = $1
          AND expires_at > now()
        ",
        table.qualified()
    );
    if for_update {
        sql.push_str("FOR UPDATE;");
    }
    let row = sqlx::query(&sql)
        .bind(job_id)
        .fetch_optional(executor)
        .await?;
    if let Some(row) = row {
        let state_json: Value = row.try_get("state")?;
        let state: JobState = serde_json::from_value(state_json)?;
        let message_id: Option<i64> = row.try_get("message_id")?;
        Ok(Some(JobRecord { state, message_id }))
    } else {
        Ok(None)
    }
}

async fn message_exists<'a, E>(
    executor: E,
    table: &QueueTableName,
    message_id: i64,
) -> Result<bool, ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let sql = format!(
        "
        SELECT 1
        FROM {}
        WHERE msg_id = $1
        LIMIT 1;
        ",
        table.qualified()
    );
    let row = sqlx::query(&sql)
        .bind(message_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.is_some())
}

async fn save_job<'a, E>(
    executor: E,
    table: &JobTableName,
    job: &JobState,
    ttl_seconds: u64,
    message_update: MessageUpdate,
) -> Result<(), ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let ttl = ttl_to_i64(ttl_seconds)?;
    let payload = serde_json::to_value(job)?;
    match message_update {
        MessageUpdate::Leave => {
            let sql = format!(
                "
                UPDATE {}
                SET
                    state = $2::jsonb,
                    expires_at = now() + ($3::bigint * INTERVAL '1 second')
                WHERE job_id = $1;
                ",
                table.qualified()
            );
            let result = sqlx::query(&sql)
                .bind(&job.job_id)
                .bind(payload)
                .bind(ttl)
                .execute(executor)
                .await?;
            if result.rows_affected() == 0 {
                Err(ProverError::InvalidInput(format!(
                    "job {} not found",
                    job.job_id
                )))
            } else {
                Ok(())
            }
        }
        MessageUpdate::Set(value) => {
            let sql = format!(
                "
                UPDATE {}
                SET
                    state = $2::jsonb,
                    expires_at = now() + ($3::bigint * INTERVAL '1 second'),
                    message_id = $4
                WHERE job_id = $1;
                ",
                table.qualified()
            );
            let result = sqlx::query(&sql)
                .bind(&job.job_id)
                .bind(payload)
                .bind(ttl)
                .bind(value)
                .execute(executor)
                .await?;
            if result.rows_affected() == 0 {
                Err(ProverError::InvalidInput(format!(
                    "job {} not found",
                    job.job_id
                )))
            } else {
                Ok(())
            }
        }
    }
}

async fn set_message_binding<'a, E>(
    executor: E,
    table: &JobTableName,
    job_id: &str,
    message_id: Option<i64>,
) -> Result<(), ProverError>
where
    E: Executor<'a, Database = Postgres>,
{
    let sql = format!(
        "
        UPDATE {}
        SET message_id = $2
        WHERE job_id = $1;
        ",
        table.qualified()
    );
    sqlx::query(&sql)
        .bind(job_id)
        .bind(message_id)
        .execute(executor)
        .await?;
    Ok(())
}

fn ttl_to_i64(ttl: u64) -> Result<i64, ProverError> {
    i64::try_from(ttl).map_err(|_| {
        ProverError::Config(format!(
            "job_ttl_seconds value {ttl} is too large for Postgres interval arithmetic"
        ))
    })
}
