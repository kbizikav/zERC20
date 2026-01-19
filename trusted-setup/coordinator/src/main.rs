use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use actix_web::{web, App, HttpResponse, HttpServer, ResponseError};
use anyhow::{Context, Result};
use ark_bn254::Bn254;
use ark_serialize::CanonicalDeserialize;
use arkworks_phase2::{transcript::Transcript, utils::serialize_uncompressed};
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::{
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
    Client as S3Client,
};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

use trusted_setup_common::{
    contribution_key, initial_transcript_path as default_initial_transcript_path, latest_key,
    transcript_key, verify_transcript_from_initial, CeremonyCircuit, LatestMetadata,
};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl ResponseError for ApiError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        use actix_web::http::StatusCode;
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Internal(_) | ApiError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let status = self.status_code();
        HttpResponse::build(status).json(ErrorResponse {
            error: self.to_string(),
            code: status.as_u16(),
        })
    }
}

// ============================================================================
// Configuration
// ============================================================================

#[derive(Clone)]
struct Config {
    listen_addr: String,
    database_url: String,
    s3_bucket: String,
    s3_prefix: String,
    presign_ttl: Duration,
    lease_ttl: Duration,
    cleanup_interval: Duration,
    skip_transcript_verification: bool,
}

impl Config {
    fn from_env() -> Result<Self> {
        let listen_addr = std::env::var("TRUSTED_SETUP_COORDINATOR_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let database_url = std::env::var("TRUSTED_SETUP_SQLITE_PATH").unwrap_or_else(|_| {
            workspace_root()
                .join("trusted-setup")
                .join("coordinator")
                .join("coordinator.sqlite")
                .display()
                .to_string()
        });

        let s3_bucket = std::env::var("TRUSTED_SETUP_S3_BUCKET")
            .context("TRUSTED_SETUP_S3_BUCKET is required")?;
        let s3_prefix = std::env::var("TRUSTED_SETUP_S3_PREFIX").unwrap_or_default();
        let presign_ttl = std::env::var("TRUSTED_SETUP_S3_PRESIGN_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(7200)); // 120 minutes (same as lease_ttl)
        let lease_ttl = std::env::var("TRUSTED_SETUP_LEASE_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(7200)); // 120 minutes
        let cleanup_interval = std::env::var("TRUSTED_SETUP_CLEANUP_INTERVAL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));
        let skip_transcript_verification =
            std::env::var("TRUSTED_SETUP_SKIP_TRANSCRIPT_VERIFICATION")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false);

        Ok(Self {
            listen_addr,
            database_url,
            s3_bucket,
            s3_prefix,
            presign_ttl,
            lease_ttl,
            cleanup_interval,
            skip_transcript_verification,
        })
    }
}

// ============================================================================
// Application State
// ============================================================================

use std::collections::HashMap;

struct AppState {
    config: Config,
    db: SqlitePool,
    s3: Storage,
    /// Cached initial transcripts per circuit (loaded on-demand at init_ceremony)
    initial_transcripts: RwLock<HashMap<CeremonyCircuit, Arc<Transcript<Bn254>>>>,
    stats: RwLock<ServerStats>,
}

#[derive(Default, Clone, Serialize)]
struct ServerStats {
    requests_total: u64,
    contributions_total: u64,
    active_ceremonies: u64,
    started_at: u64,
}

// ============================================================================
// S3 Storage
// ============================================================================

#[derive(Clone)]
struct Storage {
    client: S3Client,
    bucket: String,
    prefix: String,
    presign_ttl: Duration,
}

impl Storage {
    fn new(client: S3Client, bucket: String, prefix: String, presign_ttl: Duration) -> Self {
        let prefix = prefix.trim_matches('/').to_string();
        Self {
            client,
            bucket,
            prefix,
            presign_ttl,
        }
    }

    fn key(&self, suffix: &str) -> String {
        if self.prefix.is_empty() {
            suffix.to_string()
        } else {
            format!("{}/{}", self.prefix, suffix)
        }
    }

    async fn presign_get(&self, key: &str) -> Result<String> {
        let full_key = self.key(key);
        let config = PresigningConfig::expires_in(self.presign_ttl)?;
        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(full_key)
            .presigned(config)
            .await
            .context("failed to presign get")?;
        Ok(presigned.uri().to_string())
    }

    async fn presign_put(&self, key: &str) -> Result<String> {
        let full_key = self.key(key);
        let config = PresigningConfig::expires_in(self.presign_ttl)?;
        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(full_key)
            .presigned(config)
            .await
            .context("failed to presign put")?;
        Ok(presigned.uri().to_string())
    }

    async fn get_bytes(&self, key: &str) -> Result<Vec<u8>> {
        let full_key = self.key(key);
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(full_key)
            .send()
            .await
            .context("failed to get object")?;
        let data = resp
            .body
            .collect()
            .await
            .context("failed to read object body")?;
        Ok(data.into_bytes().to_vec())
    }

    /// Upload bytes to S3, using multipart upload for large files (>100MB).
    async fn put_bytes(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
        const MULTIPART_THRESHOLD: usize = 100 * 1024 * 1024; // 100MB
        const PART_SIZE: usize = 100 * 1024 * 1024; // 100MB per part

        if bytes.len() < MULTIPART_THRESHOLD {
            // Use simple put for small files
            let full_key = self.key(key);
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(full_key)
                .content_type(content_type)
                .body(ByteStream::from(bytes))
                .send()
                .await
                .context("failed to upload object")?;
        } else {
            // Use multipart upload for large files
            self.put_bytes_multipart(key, bytes, content_type, PART_SIZE)
                .await?;
        }
        Ok(())
    }

    /// Upload bytes using S3 multipart upload.
    async fn put_bytes_multipart(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
        part_size: usize,
    ) -> Result<()> {
        let full_key = self.key(key);
        let total_size = bytes.len();
        let num_parts = (total_size + part_size - 1) / part_size;

        log::info!(
            "Starting multipart upload: {} ({} bytes, {} parts)",
            full_key,
            total_size,
            num_parts
        );

        // Create multipart upload
        let create_resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type(content_type)
            .send()
            .await
            .context("failed to create multipart upload")?;

        let upload_id = create_resp
            .upload_id()
            .context("no upload_id in response")?;

        let mut completed_parts: Vec<CompletedPart> = Vec::with_capacity(num_parts);

        // Upload each part
        for part_number in 1..=num_parts {
            let start = (part_number - 1) * part_size;
            let end = std::cmp::min(start + part_size, total_size);
            let part_bytes = bytes[start..end].to_vec();

            log::info!(
                "Uploading part {}/{} ({} bytes)",
                part_number,
                num_parts,
                part_bytes.len()
            );

            let upload_resp = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(&full_key)
                .upload_id(upload_id)
                .part_number(part_number as i32)
                .body(ByteStream::from(part_bytes))
                .send()
                .await;

            match upload_resp {
                Ok(resp) => {
                    let etag = resp.e_tag().context("no etag in upload_part response")?;
                    completed_parts.push(
                        CompletedPart::builder()
                            .part_number(part_number as i32)
                            .e_tag(etag)
                            .build(),
                    );
                }
                Err(e) => {
                    // Abort multipart upload on failure
                    log::error!(
                        "Part {} upload failed, aborting multipart upload: {}",
                        part_number,
                        e
                    );
                    let _ = self
                        .client
                        .abort_multipart_upload()
                        .bucket(&self.bucket)
                        .key(&full_key)
                        .upload_id(upload_id)
                        .send()
                        .await;
                    return Err(e).context("failed to upload part")?;
                }
            }
        }

        // Complete multipart upload
        let completed_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .upload_id(upload_id)
            .multipart_upload(completed_upload)
            .send()
            .await
            .context("failed to complete multipart upload")?;

        log::info!("Multipart upload completed: {}", full_key);
        Ok(())
    }

    /// Start a multipart upload and return the upload_id.
    async fn start_multipart_upload(&self, key: &str, content_type: &str) -> Result<String> {
        let full_key = self.key(key);
        let resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .content_type(content_type)
            .send()
            .await
            .context("failed to create multipart upload")?;

        resp.upload_id()
            .map(|s| s.to_string())
            .context("no upload_id in response")
    }

    /// Generate a presigned URL for uploading a specific part.
    async fn presign_upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
    ) -> Result<String> {
        let full_key = self.key(key);
        let config = PresigningConfig::expires_in(self.presign_ttl)?;
        let presigned = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(&full_key)
            .upload_id(upload_id)
            .part_number(part_number)
            .presigned(config)
            .await
            .context("failed to presign upload_part")?;
        Ok(presigned.uri().to_string())
    }

    /// Complete a multipart upload.
    async fn complete_multipart_upload(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<(i32, String)>, // (part_number, etag)
    ) -> Result<()> {
        let full_key = self.key(key);
        let completed_parts: Vec<CompletedPart> = parts
            .into_iter()
            .map(|(part_number, etag)| {
                CompletedPart::builder()
                    .part_number(part_number)
                    .e_tag(etag)
                    .build()
            })
            .collect();

        let completed_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(completed_parts))
            .build();

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .upload_id(upload_id)
            .multipart_upload(completed_upload)
            .send()
            .await
            .context("failed to complete multipart upload")?;

        Ok(())
    }

    /// Abort a multipart upload.
    async fn abort_multipart_upload(&self, key: &str, upload_id: &str) -> Result<()> {
        let full_key = self.key(key);
        self.client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(&full_key)
            .upload_id(upload_id)
            .send()
            .await
            .context("failed to abort multipart upload")?;
        Ok(())
    }
}

// ============================================================================
// API Request/Response Types
// ============================================================================

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: u16,
}

#[derive(Serialize)]
struct ParticipateResponse {
    lease_id: String,
    participant_id: String,
    circuit: String,
    step: u64,
    expires_at: u64,
    expires_in_seconds: u64,
    input_url: String,
    output_url: String,
    contribution_url: String,
}

// Multipart upload types
#[derive(Deserialize)]
struct MultipartStartRequest {
    lease_id: String,
    key_type: String, // "transcript" or "contribution"
}

#[derive(Serialize)]
struct MultipartStartResponse {
    upload_id: String,
    key: String,
}

#[derive(Deserialize)]
struct MultipartPresignRequest {
    lease_id: String,
    upload_id: String,
    key: String,
    part_number: i32,
}

#[derive(Serialize)]
struct MultipartPresignResponse {
    url: String,
}

#[derive(Deserialize)]
struct MultipartPartInfo {
    part_number: i32,
    etag: String,
}

#[derive(Deserialize)]
struct MultipartCompleteRequest {
    lease_id: String,
    upload_id: String,
    key: String,
    parts: Vec<MultipartPartInfo>,
}

#[derive(Deserialize)]
struct MultipartAbortRequest {
    lease_id: String,
    upload_id: String,
    key: String,
}

#[derive(Deserialize)]
struct SubmitRequest {
    lease_id: String,
    participant_id: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
}

#[derive(Serialize)]
struct CeremonyStatus {
    id: String,
    circuit: String,
    status: String,
    current_step: u64,
    current_head_key: String,
    has_active_lease: bool,
    active_lease_expires_at: Option<u64>,
    total_contributions: u64,
    created_at: u64,
    updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

#[derive(Serialize)]
struct LeaseStatus {
    id: String,
    ceremony_id: String,
    participant_id: String,
    status: String,
    step: u64,
    started_at: u64,
    expires_at: u64,
    remaining_seconds: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

#[derive(Serialize)]
struct CeremonyStats {
    ceremony_id: String,
    circuit: String,
    current_step: u64,
    total_contributions: u64,
    completed_contributions: u64,
    pending_contributions: u64,
    expired_contributions: u64,
    average_contribution_time_seconds: Option<f64>,
}

#[derive(Serialize)]
struct ListCeremoniesResponse {
    ceremonies: Vec<CeremonyStatus>,
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    log::info!("Starting coordinator server...");

    let config = Config::from_env()?;

    log::info!("Connecting to database: {}", config.database_url);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .with_context(|| format!("failed to connect to {}", config.database_url))?;

    init_db(&pool).await?;
    log::info!("Database initialized");

    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    #[allow(deprecated)]
    let aws_config = aws_config::from_env().region(region_provider).load().await;
    let s3 = Storage::new(
        S3Client::new(&aws_config),
        config.s3_bucket.clone(),
        config.s3_prefix.clone(),
        config.presign_ttl,
    );
    log::info!("S3 client initialized (bucket: {})", config.s3_bucket);

    let stats = RwLock::new(ServerStats {
        started_at: unix_seconds(),
        ..Default::default()
    });

    let state = web::Data::new(AppState {
        config: config.clone(),
        db: pool.clone(),
        s3,
        initial_transcripts: RwLock::new(HashMap::new()),
        stats,
    });

    // Start background cleanup task
    let cleanup_state = state.clone();
    let cleanup_interval = config.cleanup_interval;
    tokio::spawn(async move {
        run_background_cleanup(cleanup_state, cleanup_interval).await;
    });

    log::info!("Server listening on {}", config.listen_addr);
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            // Health and status endpoints
            .route("/health", web::get().to(health_check))
            .route("/api/stats", web::get().to(get_server_stats))
            // Ceremony management
            .route("/api/ceremonies", web::get().to(list_ceremonies))
            .route(
                "/api/ceremonies/{ceremony_id}",
                web::get().to(get_ceremony_status),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/stats",
                web::get().to(get_ceremony_stats),
            )
            .route(
                "/api/ceremonies/init/{circuit}",
                web::post().to(init_ceremony),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/participate",
                web::get().to(participate),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/submit",
                web::post().to(submit),
            )
            // Lease management
            .route(
                "/api/ceremonies/{ceremony_id}/leases/{lease_id}",
                web::get().to(get_lease_status),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/expire-lease",
                web::post().to(expire_lease),
            )
            // Multipart upload
            .route(
                "/api/ceremonies/{ceremony_id}/multipart/start",
                web::post().to(multipart_start),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/multipart/presign",
                web::post().to(multipart_presign),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/multipart/complete",
                web::post().to(multipart_complete),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/multipart/abort",
                web::post().to(multipart_abort),
            )
    })
    .bind(&config.listen_addr)
    .with_context(|| format!("failed to bind {}", config.listen_addr))?
    .run()
    .await
    .context("server error")?;

    Ok(())
}

// ============================================================================
// Background Cleanup Task
// ============================================================================

async fn run_background_cleanup(state: web::Data<AppState>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;

        let now = unix_seconds();
        match cleanup_expired_leases(&state.db, now).await {
            Ok(count) => {
                if count > 0 {
                    log::info!("Cleaned up {} expired leases", count);
                }
            }
            Err(e) => {
                log::error!("Failed to cleanup expired leases: {}", e);
            }
        }

        // Update stats
        if let Ok(active_count) = count_active_ceremonies(&state.db).await {
            let mut stats = state.stats.write().await;
            stats.active_ceremonies = active_count;
        }
    }
}

async fn cleanup_expired_leases(pool: &SqlitePool, now: u64) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE leases SET status = 'expired' WHERE status = 'active' AND expires_at <= ?",
    )
    .bind(now as i64)
    .execute(pool)
    .await?;

    // Also update corresponding contributions
    sqlx::query(
        "UPDATE contributions SET status = 'expired', updated_at = ?
         WHERE status = 'pending' AND id IN (
             SELECT c.id FROM contributions c
             JOIN leases l ON c.ceremony_id = l.ceremony_id AND c.step = l.step
             WHERE l.status = 'expired'
         )",
    )
    .bind(now as i64)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

async fn count_active_ceremonies(pool: &SqlitePool) -> Result<u64> {
    let row = sqlx::query("SELECT COUNT(*) as count FROM ceremonies")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("count") as u64)
}

// ============================================================================
// Database Initialization
// ============================================================================

async fn init_db(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ceremonies (
            id TEXT PRIMARY KEY,
            circuit TEXT NOT NULL,
            current_head_key TEXT NOT NULL,
            step INTEGER NOT NULL,
            lease_ttl_seconds INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            error_message TEXT
        )",
    )
    .execute(pool)
    .await?;

    // Add status and error_message columns if they don't exist (for existing databases)
    let _ = sqlx::query("ALTER TABLE ceremonies ADD COLUMN status TEXT NOT NULL DEFAULT 'active'")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE ceremonies ADD COLUMN error_message TEXT")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS leases (
            id TEXT PRIMARY KEY,
            ceremony_id TEXT NOT NULL,
            participant_id TEXT NOT NULL,
            status TEXT NOT NULL,
            input_key TEXT NOT NULL,
            output_key TEXT NOT NULL,
            contribution_key TEXT NOT NULL,
            step INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            error_message TEXT
        )",
    )
    .execute(pool)
    .await?;

    // Add error_message column if it doesn't exist (for existing databases)
    let _ = sqlx::query("ALTER TABLE leases ADD COLUMN error_message TEXT")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS contributions (
            id TEXT PRIMARY KEY,
            ceremony_id TEXT NOT NULL,
            participant_id TEXT NOT NULL,
            step INTEGER NOT NULL,
            input_key TEXT NOT NULL,
            output_key TEXT NOT NULL,
            contribution_key TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    // Create indexes for better query performance
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_leases_ceremony_status ON leases(ceremony_id, status)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_contributions_ceremony ON contributions(ceremony_id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ============================================================================
// Health and Stats Endpoints
// ============================================================================

async fn health_check(state: web::Data<AppState>) -> HttpResponse {
    let stats = state.stats.read().await;
    let uptime = unix_seconds().saturating_sub(stats.started_at);

    HttpResponse::Ok().json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: uptime,
    })
}

async fn get_server_stats(state: web::Data<AppState>) -> HttpResponse {
    let stats = state.stats.read().await;
    HttpResponse::Ok().json(stats.clone())
}

// ============================================================================
// Ceremony Management Endpoints
// ============================================================================

async fn list_ceremonies(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let now = unix_seconds();

    let rows = sqlx::query(
        "SELECT c.id, c.circuit, c.status, c.error_message, c.step, c.current_head_key, c.created_at, c.updated_at,
                (SELECT COUNT(*) FROM contributions WHERE ceremony_id = c.id AND status = 'completed') as total_contributions,
                (SELECT expires_at FROM leases WHERE ceremony_id = c.id AND status = 'active' ORDER BY started_at DESC LIMIT 1) as active_lease_expires
         FROM ceremonies c
         ORDER BY c.created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let ceremonies: Vec<CeremonyStatus> = rows
        .into_iter()
        .map(|row| {
            let active_lease_expires: Option<i64> = row.get("active_lease_expires");
            let has_active_lease = active_lease_expires
                .map(|e| e as u64 > now)
                .unwrap_or(false);

            CeremonyStatus {
                id: row.get("id"),
                circuit: row.get("circuit"),
                status: row.get("status"),
                current_step: row.get::<i64, _>("step") as u64,
                current_head_key: row.get("current_head_key"),
                has_active_lease,
                active_lease_expires_at: active_lease_expires.map(|e| e as u64),
                total_contributions: row.get::<i64, _>("total_contributions") as u64,
                created_at: row.get::<i64, _>("created_at") as u64,
                updated_at: row.get::<i64, _>("updated_at") as u64,
                error_message: row.get("error_message"),
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(ListCeremoniesResponse { ceremonies }))
}

async fn get_ceremony_status(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();
    let now = unix_seconds();

    let row = sqlx::query(
        "SELECT c.id, c.circuit, c.status, c.error_message, c.step, c.current_head_key, c.created_at, c.updated_at,
                (SELECT COUNT(*) FROM contributions WHERE ceremony_id = c.id AND status = 'completed') as total_contributions,
                (SELECT expires_at FROM leases WHERE ceremony_id = c.id AND status = 'active' ORDER BY started_at DESC LIMIT 1) as active_lease_expires
         FROM ceremonies c
         WHERE c.id = ?",
    )
    .bind(&ceremony_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("ceremony {} not found", ceremony_id)))?;

    let active_lease_expires: Option<i64> = row.get("active_lease_expires");
    let has_active_lease = active_lease_expires
        .map(|e| e as u64 > now)
        .unwrap_or(false);

    let status = CeremonyStatus {
        id: row.get("id"),
        circuit: row.get("circuit"),
        status: row.get("status"),
        current_step: row.get::<i64, _>("step") as u64,
        current_head_key: row.get("current_head_key"),
        has_active_lease,
        active_lease_expires_at: active_lease_expires.map(|e| e as u64),
        total_contributions: row.get::<i64, _>("total_contributions") as u64,
        created_at: row.get::<i64, _>("created_at") as u64,
        updated_at: row.get::<i64, _>("updated_at") as u64,
        error_message: row.get("error_message"),
    };

    Ok(HttpResponse::Ok().json(status))
}

async fn get_ceremony_stats(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    let ceremony = sqlx::query("SELECT circuit, step FROM ceremonies WHERE id = ?")
        .bind(&ceremony_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("ceremony {} not found", ceremony_id)))?;

    let stats_row = sqlx::query(
        "SELECT
            COUNT(*) as total,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
            SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
            SUM(CASE WHEN status = 'expired' THEN 1 ELSE 0 END) as expired
         FROM contributions WHERE ceremony_id = ?",
    )
    .bind(&ceremony_id)
    .fetch_one(&state.db)
    .await?;

    // Calculate average contribution time from completed contributions
    let avg_time: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(l.expires_at - l.started_at - (l.expires_at - c.updated_at)) as avg_time
         FROM contributions c
         JOIN leases l ON c.ceremony_id = l.ceremony_id AND c.step = l.step
         WHERE c.ceremony_id = ? AND c.status = 'completed'",
    )
    .bind(&ceremony_id)
    .fetch_one(&state.db)
    .await
    .ok()
    .flatten();

    let stats = CeremonyStats {
        ceremony_id: ceremony_id.clone(),
        circuit: ceremony.get("circuit"),
        current_step: ceremony.get::<i64, _>("step") as u64,
        total_contributions: stats_row.get::<i64, _>("total") as u64,
        completed_contributions: stats_row.get::<i64, _>("completed") as u64,
        pending_contributions: stats_row.get::<i64, _>("pending") as u64,
        expired_contributions: stats_row.get::<i64, _>("expired") as u64,
        average_contribution_time_seconds: avg_time,
    };

    Ok(HttpResponse::Ok().json(stats))
}

async fn get_lease_status(
    state: web::Data<AppState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse, ApiError> {
    let (ceremony_id, lease_id) = path.into_inner();
    let now = unix_seconds();

    let row = sqlx::query(
        "SELECT id, ceremony_id, participant_id, status, step, started_at, expires_at, error_message
         FROM leases WHERE id = ? AND ceremony_id = ?",
    )
    .bind(&lease_id)
    .bind(&ceremony_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("lease {} not found", lease_id)))?;

    let expires_at: i64 = row.get("expires_at");
    let remaining = expires_at - now as i64;

    let status = LeaseStatus {
        id: row.get("id"),
        ceremony_id: row.get("ceremony_id"),
        participant_id: row.get("participant_id"),
        status: row.get("status"),
        step: row.get::<i64, _>("step") as u64,
        started_at: row.get::<i64, _>("started_at") as u64,
        expires_at: expires_at as u64,
        remaining_seconds: remaining,
        error_message: row.get("error_message"),
    };

    Ok(HttpResponse::Ok().json(status))
}

/// Force expire all active leases for a ceremony (admin endpoint).
async fn expire_lease(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();
    let now = unix_seconds();

    log::info!("Force expiring active leases for ceremony {}", ceremony_id);

    // Update all active leases to expired
    let result = sqlx::query(
        "UPDATE leases SET status = 'expired', expires_at = ?
         WHERE ceremony_id = ? AND status = 'active'",
    )
    .bind(now as i64)
    .bind(&ceremony_id)
    .execute(&state.db)
    .await?;

    let expired_count = result.rows_affected();
    log::info!(
        "Force expired {} active lease(s) for ceremony {}",
        expired_count,
        ceremony_id
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "ceremony_id": ceremony_id,
        "expired_count": expired_count,
        "message": format!("Expired {} active lease(s)", expired_count)
    })))
}

// ============================================================================
// Ceremony Lifecycle Endpoints
// ============================================================================

async fn init_ceremony(
    state: web::Data<AppState>,
    circuit_path: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let circuit_str = circuit_path.into_inner();
    let ceremony_id = Uuid::new_v4().to_string();

    log::info!(
        "Initializing ceremony {} with circuit {}",
        ceremony_id,
        circuit_str
    );

    // Increment request counter
    {
        let mut stats = state.stats.write().await;
        stats.requests_total += 1;
    }

    let circuit =
        CeremonyCircuit::parse(&circuit_str).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let now = unix_seconds();
    let step = 0u64;
    let tkey = transcript_key(&ceremony_id, step);

    // Create ceremony with "initializing" status
    insert_ceremony(
        &state.db,
        &ceremony_id,
        circuit,
        &tkey,
        "initializing",
        now,
        &state.config,
    )
    .await?;

    // Spawn background task for heavy initialization
    let state_clone = state.clone();
    let ceremony_id_clone = ceremony_id.clone();
    let tkey_clone = tkey.clone();
    tokio::spawn(async move {
        let result =
            process_init_ceremony(&state_clone, &ceremony_id_clone, circuit, &tkey_clone).await;

        match result {
            Ok(_) => {
                log::info!(
                    "Background ceremony initialization completed successfully for {}",
                    ceremony_id_clone
                );
            }
            Err(e) => {
                log::error!(
                    "Background ceremony initialization failed for {}: {}",
                    ceremony_id_clone,
                    e
                );
                // Update ceremony with error
                let _ = sqlx::query(
                    "UPDATE ceremonies SET status = 'failed', error_message = ? WHERE id = ?",
                )
                .bind(e.to_string())
                .bind(&ceremony_id_clone)
                .execute(&state_clone.db)
                .await;
            }
        }
    });

    // Return immediately with 202 Accepted
    Ok(HttpResponse::Accepted().json(serde_json::json!({
        "ceremony_id": ceremony_id,
        "status": "initializing",
        "message": "ceremony initialization started, poll status endpoint for completion"
    })))
}

/// Process ceremony initialization in background (heavy work)
async fn process_init_ceremony(
    state: &web::Data<AppState>,
    ceremony_id: &str,
    circuit: CeremonyCircuit,
    tkey: &str,
) -> Result<(), anyhow::Error> {
    // Load initial transcript from cache or file
    let initial_transcript = get_or_load_initial_transcript(state, circuit)
        .await
        .map_err(|e| anyhow::anyhow!("failed to load initial transcript: {}", e))?;

    log::info!(
        "Using initial transcript for circuit {:?}...",
        circuit.as_str()
    );

    log::info!("Serializing transcript...");
    let transcript_bytes = serialize_uncompressed(initial_transcript.as_ref())
        .map_err(|e| anyhow::anyhow!("failed to serialize transcript: {}", e))?;
    log::info!("Transcript serialized ({} bytes)", transcript_bytes.len());

    let lkey = latest_key(ceremony_id);

    // Upload to S3
    log::info!("Uploading transcript to S3: {}...", tkey);
    state
        .s3
        .put_bytes(tkey, transcript_bytes, "application/octet-stream")
        .await?;
    log::info!("Transcript uploaded successfully");

    let now = unix_seconds();
    let latest = LatestMetadata {
        step: 0,
        transcript_key: tkey.to_string(),
        contribution_key: None,
        updated_at: now,
    };

    let latest_bytes = serde_json::to_vec(&latest)?;

    state
        .s3
        .put_bytes(&lkey, latest_bytes, "application/json")
        .await?;

    // Update ceremony status to active
    sqlx::query("UPDATE ceremonies SET status = 'active', updated_at = ? WHERE id = ?")
        .bind(now as i64)
        .bind(ceremony_id)
        .execute(&state.db)
        .await?;

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.active_ceremonies += 1;
    }

    Ok(())
}

async fn participate(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    // Increment request counter
    {
        let mut stats = state.stats.write().await;
        stats.requests_total += 1;
    }

    let now = unix_seconds();

    let mut tx = state.db.begin().await?;

    let ceremony_row =
        sqlx::query("SELECT circuit, current_head_key, step, status FROM ceremonies WHERE id = ?")
            .bind(&ceremony_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("ceremony {} not found", ceremony_id)))?;

    let ceremony_status: String = ceremony_row.get("status");
    if ceremony_status != "active" {
        return Err(ApiError::Conflict(format!(
            "ceremony is not active (status: {})",
            ceremony_status
        )));
    }

    let circuit_str: String = ceremony_row.get("circuit");

    // Expire any active leases that are past their expiry time
    expire_active_lease(&mut tx, &ceremony_id, now).await?;

    // Check if there's still an active (non-expired) lease
    if let Some(expires_at) = active_lease_expires_at(&mut tx, &ceremony_id).await? {
        if expires_at > now {
            let remaining = expires_at - now;
            return Err(ApiError::Conflict(format!(
                "active lease exists, expires in {} seconds",
                remaining
            )));
        }
    }

    let current_head_key: String = ceremony_row.get("current_head_key");
    let current_step: i64 = ceremony_row.get("step");
    let next_step = (current_step + 1) as u64;

    let lease_id = Uuid::new_v4().to_string();
    let participant_id = Uuid::new_v4().to_string();
    let output_key = transcript_key(&ceremony_id, next_step);
    let contrib_key = contribution_key(&ceremony_id, next_step);

    let expires_at = now + state.config.lease_ttl.as_secs();

    sqlx::query(
        "INSERT INTO leases (id, ceremony_id, participant_id, status, input_key, output_key, contribution_key, step, started_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&lease_id)
    .bind(&ceremony_id)
    .bind(&participant_id)
    .bind("active")
    .bind(&current_head_key)
    .bind(&output_key)
    .bind(&contrib_key)
    .bind(next_step as i64)
    .bind(now as i64)
    .bind(expires_at as i64)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO contributions (id, ceremony_id, participant_id, step, input_key, output_key, contribution_key, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&ceremony_id)
    .bind(&participant_id)
    .bind(next_step as i64)
    .bind(&current_head_key)
    .bind(&output_key)
    .bind(&contrib_key)
    .bind("pending")
    .bind(now as i64)
    .bind(now as i64)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let input_url = state.s3.presign_get(&current_head_key).await?;
    let output_url = state.s3.presign_put(&output_key).await?;
    let contribution_url = state.s3.presign_put(&contrib_key).await?;

    let expires_in = expires_at - now;

    Ok(HttpResponse::Ok().json(ParticipateResponse {
        lease_id,
        participant_id,
        circuit: circuit_str,
        step: next_step,
        expires_at,
        expires_in_seconds: expires_in,
        input_url,
        output_url,
        contribution_url,
    }))
}

async fn submit(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<SubmitRequest>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();
    log::info!(
        "Processing submission for ceremony {} from participant {}",
        ceremony_id,
        body.participant_id
    );

    // Increment request counter
    {
        let mut stats = state.stats.write().await;
        stats.requests_total += 1;
    }

    let now = unix_seconds();

    // Phase 1: Validate lease and mark as processing (quick DB operation)
    let (input_key, output_key, contrib_key, step, circuit) = {
        let lease_row = sqlx::query(
            "SELECT l.ceremony_id, l.participant_id, l.status, l.input_key, l.output_key, l.contribution_key, l.step, l.expires_at, c.circuit
             FROM leases l
             JOIN ceremonies c ON l.ceremony_id = c.id
             WHERE l.id = ?",
        )
        .bind(&body.lease_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("lease not found".to_string()))?;

        let lease_ceremony: String = lease_row.get("ceremony_id");
        if lease_ceremony != ceremony_id {
            return Err(ApiError::BadRequest("lease ceremony mismatch".to_string()));
        }
        let participant_id: String = lease_row.get("participant_id");
        if participant_id != body.participant_id {
            return Err(ApiError::BadRequest("participant mismatch".to_string()));
        }
        let status: String = lease_row.get("status");
        if status == "processing" {
            // Already processing, return 202 immediately
            return Ok(HttpResponse::Accepted().json(serde_json::json!({
                "message": "submission is being processed",
                "lease_id": body.lease_id,
                "status": "processing"
            })));
        }
        if status != "active" {
            return Err(ApiError::Conflict(format!(
                "lease is not active (status: {})",
                status
            )));
        }
        let expires_at: i64 = lease_row.get("expires_at");
        if expires_at as u64 <= now {
            sqlx::query("UPDATE leases SET status = ? WHERE id = ?")
                .bind("expired")
                .bind(&body.lease_id)
                .execute(&state.db)
                .await?;
            return Err(ApiError::Conflict("lease expired".to_string()));
        }

        let input_key: String = lease_row.get("input_key");
        let output_key: String = lease_row.get("output_key");
        let contrib_key: String = lease_row.get("contribution_key");
        let step: i64 = lease_row.get("step");
        let circuit_str: String = lease_row.get("circuit");
        let circuit = CeremonyCircuit::parse(&circuit_str).map_err(ApiError::Internal)?;

        (input_key, output_key, contrib_key, step, circuit)
    };

    // Mark lease as processing
    sqlx::query("UPDATE leases SET status = 'processing', error_message = NULL WHERE id = ?")
        .bind(&body.lease_id)
        .execute(&state.db)
        .await?;

    // Spawn background task for heavy verification
    let state_clone = state.clone();
    let lease_id = body.lease_id.clone();
    let ceremony_id_clone = ceremony_id.clone();
    tokio::spawn(async move {
        let result = process_submission(
            &state_clone,
            &ceremony_id_clone,
            &lease_id,
            input_key,
            output_key,
            contrib_key,
            step,
            circuit,
        )
        .await;

        match result {
            Ok(_) => {
                log::info!(
                    "Background submission completed successfully for ceremony {} step {}",
                    ceremony_id_clone,
                    step
                );
            }
            Err(e) => {
                log::error!(
                    "Background submission failed for ceremony {} step {}: {}",
                    ceremony_id_clone,
                    step,
                    e
                );
                // Update lease with error
                let _ = sqlx::query(
                    "UPDATE leases SET status = 'failed', error_message = ? WHERE id = ?",
                )
                .bind(e.to_string())
                .bind(&lease_id)
                .execute(&state_clone.db)
                .await;
            }
        }
    });

    // Return immediately with 202 Accepted
    Ok(HttpResponse::Accepted().json(serde_json::json!({
        "message": "submission accepted, processing in background",
        "lease_id": body.lease_id,
        "status": "processing"
    })))
}

/// Process submission in background (heavy verification work)
async fn process_submission(
    state: &web::Data<AppState>,
    ceremony_id: &str,
    lease_id: &str,
    _input_key: String,
    output_key: String,
    contrib_key: String,
    step: i64,
    circuit: CeremonyCircuit,
) -> Result<(), anyhow::Error> {
    let now = unix_seconds();

    // Skip transcript verification if configured
    if state.config.skip_transcript_verification {
        log::warn!(
            "TRUSTED_SETUP_SKIP_TRANSCRIPT_VERIFICATION is enabled - skipping transcript download and verification"
        );
    } else {
        // Get or load initial transcript from cache or file
        let initial_transcript = get_or_load_initial_transcript(state, circuit)
            .await
            .map_err(|e| anyhow::anyhow!("failed to load initial transcript: {}", e))?;

        // Download and verify transcript
        log::info!("Downloading output transcript from S3: {}...", output_key);
        let output_bytes = state.s3.get_bytes(&output_key).await?;
        log::info!(
            "Output transcript downloaded ({} bytes)",
            output_bytes.len()
        );

        log::info!("Deserializing output transcript...");
        let output_transcript = tokio::task::spawn_blocking(move || {
            Transcript::<Bn254>::deserialize_uncompressed(&output_bytes[..])
        })
        .await
        .context("spawn_blocking failed")?
        .map_err(|e| anyhow::anyhow!("invalid transcript: {}", e))?;
        log::info!("Output transcript deserialized");

        log::info!("Verifying transcript for circuit {:?}...", circuit.as_str());
        verify_transcript_from_initial(&initial_transcript, &output_transcript)
            .map_err(|e| anyhow::anyhow!("transcript verification failed: {}", e))?;
        log::info!("Transcript verification passed");

        // Verify the contribution count matches the expected step
        // This prevents replay attacks where an older transcript is submitted
        let contribution_count = output_transcript.contributions.len();
        if contribution_count != step as usize {
            return Err(anyhow::anyhow!(
                "transcript has {} contributions, expected {} for step {}",
                contribution_count,
                step,
                step
            ));
        }
        log::info!(
            "Contribution count verified: {} contributions for step {}",
            contribution_count,
            step
        );
    }

    // Upload latest.json BEFORE updating database for consistency
    let latest = LatestMetadata {
        step: step as u64,
        transcript_key: output_key.clone(),
        contribution_key: Some(contrib_key.clone()),
        updated_at: now,
    };
    let latest_bytes = serde_json::to_vec(&latest)?;

    log::info!("Updating latest metadata in S3...");
    state
        .s3
        .put_bytes(&latest_key(ceremony_id), latest_bytes, "application/json")
        .await?;

    // Update database (quick transaction after all verification passed)
    let mut tx = state.db.begin().await?;

    // Re-check lease status (might have been cancelled during verification)
    let lease_check = sqlx::query("SELECT status FROM leases WHERE id = ?")
        .bind(lease_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("lease not found"))?;
    let current_status: String = lease_check.get("status");
    if current_status != "processing" {
        return Err(anyhow::anyhow!(
            "lease is no longer processing (status: {})",
            current_status
        ));
    }

    sqlx::query(
        "UPDATE ceremonies SET current_head_key = ?, step = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&output_key)
    .bind(step)
    .bind(now as i64)
    .bind(ceremony_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE leases SET status = 'completed' WHERE id = ?")
        .bind(lease_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE contributions SET status = 'completed', updated_at = ? WHERE ceremony_id = ? AND step = ?",
    )
    .bind(now as i64)
    .bind(ceremony_id)
    .bind(step)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.contributions_total += 1;
    }

    Ok(())
}

// ============================================================================
// Multipart Upload Handlers
// ============================================================================

/// Start a multipart upload for transcript or contribution.
async fn multipart_start(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<MultipartStartRequest>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    // Verify lease is valid
    let lease = sqlx::query(
        "SELECT output_key, contribution_key, status, expires_at FROM leases WHERE id = ? AND ceremony_id = ?",
    )
    .bind(&body.lease_id)
    .bind(&ceremony_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::NotFound("lease not found".to_string()))?;

    let status: String = lease.get("status");
    let expires_at: i64 = lease.get("expires_at");

    if status != "active" {
        return Err(ApiError::BadRequest("lease is not active".to_string()));
    }
    if expires_at < unix_seconds() as i64 {
        return Err(ApiError::BadRequest("lease has expired".to_string()));
    }

    let key = match body.key_type.as_str() {
        "transcript" => lease.get::<String, _>("output_key"),
        "contribution" => lease.get::<String, _>("contribution_key"),
        _ => return Err(ApiError::BadRequest("invalid key_type".to_string())),
    };

    let upload_id = state
        .s3
        .start_multipart_upload(&key, "application/octet-stream")
        .await
        .map_err(ApiError::Internal)?;

    log::info!(
        "Started multipart upload for {} (upload_id: {})",
        key,
        upload_id
    );

    Ok(HttpResponse::Ok().json(MultipartStartResponse { upload_id, key }))
}

/// Generate presigned URL for a specific part.
async fn multipart_presign(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<MultipartPresignRequest>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    // Verify lease is valid
    let lease =
        sqlx::query("SELECT status, expires_at FROM leases WHERE id = ? AND ceremony_id = ?")
            .bind(&body.lease_id)
            .bind(&ceremony_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::NotFound("lease not found".to_string()))?;

    let status: String = lease.get("status");
    let expires_at: i64 = lease.get("expires_at");

    if status != "active" {
        return Err(ApiError::BadRequest("lease is not active".to_string()));
    }
    if expires_at < unix_seconds() as i64 {
        return Err(ApiError::BadRequest("lease has expired".to_string()));
    }

    let url = state
        .s3
        .presign_upload_part(&body.key, &body.upload_id, body.part_number)
        .await
        .map_err(ApiError::Internal)?;

    Ok(HttpResponse::Ok().json(MultipartPresignResponse { url }))
}

/// Complete a multipart upload.
async fn multipart_complete(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<MultipartCompleteRequest>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    // Verify lease is valid
    let lease =
        sqlx::query("SELECT status, expires_at FROM leases WHERE id = ? AND ceremony_id = ?")
            .bind(&body.lease_id)
            .bind(&ceremony_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::NotFound("lease not found".to_string()))?;

    let status: String = lease.get("status");
    let expires_at: i64 = lease.get("expires_at");

    if status != "active" {
        return Err(ApiError::BadRequest("lease is not active".to_string()));
    }
    if expires_at < unix_seconds() as i64 {
        return Err(ApiError::BadRequest("lease has expired".to_string()));
    }

    let parts: Vec<(i32, String)> = body
        .parts
        .iter()
        .map(|p| (p.part_number, p.etag.clone()))
        .collect();

    state
        .s3
        .complete_multipart_upload(&body.key, &body.upload_id, parts)
        .await
        .map_err(ApiError::Internal)?;

    log::info!(
        "Completed multipart upload for {} (upload_id: {})",
        body.key,
        body.upload_id
    );

    Ok(HttpResponse::Ok().finish())
}

/// Abort a multipart upload.
async fn multipart_abort(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<MultipartAbortRequest>,
) -> Result<HttpResponse, ApiError> {
    let ceremony_id = ceremony_id.into_inner();

    // Verify lease exists (don't need to check status for abort)
    let lease = sqlx::query("SELECT id FROM leases WHERE id = ? AND ceremony_id = ?")
        .bind(&body.lease_id)
        .bind(&ceremony_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::NotFound("lease not found".to_string()))?;

    let _ = lease; // Just to use the variable

    state
        .s3
        .abort_multipart_upload(&body.key, &body.upload_id)
        .await
        .map_err(ApiError::Internal)?;

    log::info!(
        "Aborted multipart upload for {} (upload_id: {})",
        body.key,
        body.upload_id
    );

    Ok(HttpResponse::Ok().finish())
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Load initial transcript from cache, or load from file and cache it.
async fn get_or_load_initial_transcript(
    state: &web::Data<AppState>,
    circuit: CeremonyCircuit,
) -> Result<Arc<Transcript<Bn254>>, ApiError> {
    // Check cache first
    {
        let cache = state.initial_transcripts.read().await;
        if let Some(transcript) = cache.get(&circuit) {
            return Ok(transcript.clone());
        }
    }

    // Not in cache, load from file
    let path = default_initial_transcript_path(circuit);
    if !path.exists() {
        return Err(ApiError::BadRequest(format!(
            "Initial transcript not found for circuit {}. Generate one using:\n\
             trusted-setup-cli generate-initial-transcript --circuit {}",
            circuit.as_str(),
            circuit.as_str()
        )));
    }

    log::info!(
        "Loading initial transcript for {} from {}...",
        circuit.as_str(),
        path.display()
    );

    // Use async file I/O for better performance
    let start = std::time::Instant::now();
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        ApiError::Internal(anyhow::anyhow!(
            "failed to read initial transcript from {}: {}",
            path.display(),
            e
        ))
    })?;
    log::info!(
        "Initial transcript file read in {:?} ({} bytes)",
        start.elapsed(),
        bytes.len()
    );

    // Deserialize in a blocking thread to avoid blocking the async runtime
    let start = std::time::Instant::now();
    let transcript = tokio::task::spawn_blocking(move || {
        Transcript::<Bn254>::deserialize_uncompressed(&bytes[..])
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("spawn_blocking failed: {}", e)))?
    .map_err(|e| {
        ApiError::Internal(anyhow::anyhow!(
            "failed to deserialize initial transcript: {}",
            e
        ))
    })?;
    log::info!(
        "Initial transcript deserialized in {:?} ({} contributions)",
        start.elapsed(),
        transcript.contributions.len()
    );

    let transcript = Arc::new(transcript);

    // Cache it
    {
        let mut cache = state.initial_transcripts.write().await;
        cache.insert(circuit, transcript.clone());
    }

    Ok(transcript)
}

async fn insert_ceremony(
    pool: &SqlitePool,
    ceremony_id: &str,
    circuit: CeremonyCircuit,
    current_head_key: &str,
    status: &str,
    now: u64,
    config: &Config,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO ceremonies (id, circuit, current_head_key, step, lease_ttl_seconds, created_at, updated_at, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ceremony_id)
    .bind(circuit.as_str())
    .bind(current_head_key)
    .bind(0i64)
    .bind(config.lease_ttl.as_secs() as i64)
    .bind(now as i64)
    .bind(now as i64)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

async fn expire_active_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ceremony_id: &str,
    now: u64,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE leases SET status = ? WHERE ceremony_id = ? AND status = ? AND expires_at <= ?",
    )
    .bind("expired")
    .bind(ceremony_id)
    .bind("active")
    .bind(now as i64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn active_lease_expires_at(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ceremony_id: &str,
) -> Result<Option<u64>, ApiError> {
    let row = sqlx::query(
        "SELECT expires_at FROM leases WHERE ceremony_id = ? AND status = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(ceremony_id)
    .bind("active")
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| row.get::<i64, _>("expires_at") as u64))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
