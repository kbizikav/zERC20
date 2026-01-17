use std::{
    cmp::max,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use actix_web::{web, App, HttpResponse, HttpServer};
use anyhow::{bail, Context, Result};
use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_grumpkin::Projective as G2;
use ark_serialize::CanonicalDeserialize;
use arkworks_phase2::{
    accumulator::Accumulator, key::PartialKey, transcript::Transcript,
    utils::serialize_uncompressed,
};
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::{presigning::PresigningConfig, primitives::ByteStream, Client as S3Client};
use folding_schemes::{
    arith::Arith,
    commitment::pedersen::Pedersen,
    commitment::CommitmentScheme,
    folding::nova::{decider_eth::DeciderEthCircuit, get_r1cs},
    folding::traits::Dummy,
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
};
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use uuid::Uuid;

use zkp::groth16::withdraw::SingleWithdrawCircuit;
use zkp::nova::{
    constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    params::FParams,
    root_nova::RootCircuit,
    withdraw_nova::WithdrawCircuit,
};
use zkp::utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config};

#[derive(Clone)]
struct Config {
    listen_addr: String,
    database_url: String,
    ptau_path: PathBuf,
    s3_bucket: String,
    s3_prefix: String,
    presign_ttl: Duration,
    lease_ttl: Duration,
    pedersen_seed: u64,
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
        let ptau_path = std::env::var("TRUSTED_SETUP_PTAU_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_ptau_path());
        let s3_bucket = std::env::var("TRUSTED_SETUP_S3_BUCKET")
            .context("TRUSTED_SETUP_S3_BUCKET is required")?;
        let s3_prefix = std::env::var("TRUSTED_SETUP_S3_PREFIX").unwrap_or_default();
        let presign_ttl = std::env::var("TRUSTED_SETUP_S3_PRESIGN_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(900));
        let lease_ttl = std::env::var("TRUSTED_SETUP_LEASE_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(900));
        let pedersen_seed = std::env::var("TRUSTED_SETUP_PEDERSEN_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(42);

        Ok(Self {
            listen_addr,
            database_url,
            ptau_path,
            s3_bucket,
            s3_prefix,
            presign_ttl,
            lease_ttl,
            pedersen_seed,
        })
    }
}

struct AppState {
    config: Config,
    db: SqlitePool,
    s3: Storage,
    accum: Arc<Accumulator<Bn254>>,
}

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

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
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
        Ok(())
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
struct InitRequest {
    circuit: String,
}

#[derive(Serialize)]
struct InitResponse {
    ceremony_id: String,
    step: u64,
    transcript_key: String,
}

#[derive(Deserialize)]
struct ParticipateRequest {
    circuit: String,
}

#[derive(Serialize)]
struct ParticipateResponse {
    lease_id: String,
    participant_id: String,
    step: u64,
    expires_at: u64,
    input_url: String,
    output_url: String,
    contribution_url: String,
}

#[derive(Deserialize)]
struct SubmitRequest {
    lease_id: String,
    participant_id: String,
}

#[derive(Serialize)]
struct SubmitResponse {
    step: u64,
    transcript_key: String,
}

#[derive(Serialize, Deserialize)]
struct LatestMetadata {
    step: u64,
    transcript_key: String,
    contribution_key: Option<String>,
    updated_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CeremonyCircuit {
    WithdrawLocal,
    WithdrawGlobal,
    DeciderRoot,
    DeciderWithdrawLocal,
    DeciderWithdrawGlobal,
}

impl CeremonyCircuit {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "withdraw_local" => Ok(Self::WithdrawLocal),
            "withdraw_global" => Ok(Self::WithdrawGlobal),
            "decider_root" | "root" => Ok(Self::DeciderRoot),
            "decider_withdraw_local" => Ok(Self::DeciderWithdrawLocal),
            "decider_withdraw_global" => Ok(Self::DeciderWithdrawGlobal),
            _ => bail!("unsupported circuit {value}"),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            CeremonyCircuit::WithdrawLocal => "withdraw_local",
            CeremonyCircuit::WithdrawGlobal => "withdraw_global",
            CeremonyCircuit::DeciderRoot => "decider_root",
            CeremonyCircuit::DeciderWithdrawLocal => "decider_withdraw_local",
            CeremonyCircuit::DeciderWithdrawGlobal => "decider_withdraw_global",
        }
    }
}

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
    let aws_config = aws_config::from_env().region(region_provider).load().await;
    let s3 = Storage::new(
        S3Client::new(&aws_config),
        config.s3_bucket.clone(),
        config.s3_prefix.clone(),
        config.presign_ttl,
    );
    log::info!("S3 client initialized (bucket: {})", config.s3_bucket);

    log::info!("Loading PTAU file from {}...", config.ptau_path.display());
    let accum = Arc::new(load_accumulator(&config.ptau_path)?);
    log::info!("PTAU file loaded successfully");

    let state = web::Data::new(AppState {
        config: config.clone(),
        db: pool.clone(),
        s3,
        accum,
    });

    log::info!("Server listening on {}", config.listen_addr);
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route(
                "/api/ceremonies/{ceremony_id}/init",
                web::post().to(init_ceremony),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/participate",
                web::post().to(participate),
            )
            .route(
                "/api/ceremonies/{ceremony_id}/submit",
                web::post().to(submit),
            )
    })
    .bind(&config.listen_addr)
    .with_context(|| format!("failed to bind {}", config.listen_addr))?
    .run()
    .await
    .context("server error")?;

    Ok(())
}

async fn init_db(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ceremonies (
            id TEXT PRIMARY KEY,
            circuit TEXT NOT NULL,
            current_head_key TEXT NOT NULL,
            step INTEGER NOT NULL,
            lease_ttl_seconds INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

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
            expires_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

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

    Ok(())
}

async fn init_ceremony(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<InitRequest>,
) -> HttpResponse {
    let ceremony_id = ceremony_id.into_inner();
    log::info!(
        "Initializing ceremony {} with circuit {}",
        ceremony_id,
        body.circuit
    );

    let circuit = match CeremonyCircuit::parse(&body.circuit) {
        Ok(circuit) => circuit,
        Err(err) => return error_response(400, err.to_string()),
    };

    if let Err(err) = ensure_ceremony_absent(&state.db, &ceremony_id).await {
        return error_response(409, err.to_string());
    }

    log::info!(
        "Building initial transcript for circuit {:?}...",
        circuit.as_str()
    );
    let transcript =
        match build_initial_transcript(&state.accum, circuit, state.config.pedersen_seed) {
            Ok(transcript) => transcript,
            Err(err) => return error_response(500, err.to_string()),
        };
    log::info!("Initial transcript built successfully");

    log::info!("Serializing transcript...");
    let transcript_bytes = match serialize_uncompressed(&transcript) {
        Ok(bytes) => bytes,
        Err(err) => return error_response(500, err.to_string()),
    };
    log::info!(
        "Transcript serialized ({} bytes)",
        transcript_bytes.len()
    );

    let step = 0u64;
    let transcript_key = transcript_key(&ceremony_id, step);
    let latest_key = latest_key(&ceremony_id);

    log::info!("Uploading transcript to S3: {}...", transcript_key);
    if let Err(err) = state
        .s3
        .put_bytes(
            &transcript_key,
            transcript_bytes,
            "application/octet-stream",
        )
        .await
    {
        return error_response(500, err.to_string());
    }
    log::info!("Transcript uploaded successfully");

    let now = unix_seconds();
    let latest = LatestMetadata {
        step,
        transcript_key: transcript_key.clone(),
        contribution_key: None,
        updated_at: now,
    };

    let latest_bytes = match serde_json::to_vec(&latest) {
        Ok(bytes) => bytes,
        Err(err) => return error_response(500, err.to_string()),
    };

    if let Err(err) = state
        .s3
        .put_bytes(&latest_key, latest_bytes, "application/json")
        .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = insert_ceremony(
        &state.db,
        &ceremony_id,
        circuit,
        &transcript_key,
        now,
        &state.config,
    )
    .await
    {
        return error_response(500, err.to_string());
    }

    log::info!("Ceremony {} initialized successfully", ceremony_id);
    HttpResponse::Ok().json(InitResponse {
        ceremony_id,
        step,
        transcript_key,
    })
}

async fn participate(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<ParticipateRequest>,
) -> HttpResponse {
    let ceremony_id = ceremony_id.into_inner();
    let circuit = match CeremonyCircuit::parse(&body.circuit) {
        Ok(circuit) => circuit,
        Err(err) => return error_response(400, err.to_string()),
    };

    let now = unix_seconds();

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => return error_response(500, err.to_string()),
    };

    let ceremony_row =
        match sqlx::query("SELECT circuit, current_head_key, step FROM ceremonies WHERE id = ?")
            .bind(&ceremony_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => return error_response(404, "ceremony not found".to_string()),
            Err(err) => return error_response(500, err.to_string()),
        };

    let stored_circuit: String = ceremony_row.get("circuit");
    if stored_circuit != circuit.as_str() {
        return error_response(400, "circuit mismatch".to_string());
    }

    if let Err(err) = expire_active_lease(&mut tx, &ceremony_id, now).await {
        return error_response(500, err.to_string());
    }

    if let Ok(Some(expires_at)) = active_lease_expires_at(&mut tx, &ceremony_id).await {
        if expires_at > now {
            return error_response(409, "active lease exists".to_string());
        }
    }

    let current_head_key: String = ceremony_row.get("current_head_key");
    let current_step: i64 = ceremony_row.get("step");
    let next_step = (current_step + 1) as u64;

    let lease_id = Uuid::new_v4().to_string();
    let participant_id = Uuid::new_v4().to_string();
    let output_key = transcript_key(&ceremony_id, next_step);
    let contribution_key = contribution_key(&ceremony_id, next_step);

    let expires_at = now + state.config.lease_ttl.as_secs() as u64;
    if let Err(err) = sqlx::query(
        "INSERT INTO leases (id, ceremony_id, participant_id, status, input_key, output_key, contribution_key, step, started_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&lease_id)
    .bind(&ceremony_id)
    .bind(&participant_id)
    .bind("active")
    .bind(&current_head_key)
    .bind(&output_key)
    .bind(&contribution_key)
    .bind(next_step as i64)
    .bind(now as i64)
    .bind(expires_at as i64)
    .execute(&mut *tx)
    .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO contributions (id, ceremony_id, participant_id, step, input_key, output_key, contribution_key, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&ceremony_id)
    .bind(&participant_id)
    .bind(next_step as i64)
    .bind(&current_head_key)
    .bind(&output_key)
    .bind(&contribution_key)
    .bind("pending")
    .bind(now as i64)
    .bind(now as i64)
    .execute(&mut *tx)
    .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = tx.commit().await {
        return error_response(500, err.to_string());
    }

    let input_url = match state.s3.presign_get(&current_head_key).await {
        Ok(url) => url,
        Err(err) => return error_response(500, err.to_string()),
    };
    let output_url = match state.s3.presign_put(&output_key).await {
        Ok(url) => url,
        Err(err) => return error_response(500, err.to_string()),
    };
    let contribution_url = match state.s3.presign_put(&contribution_key).await {
        Ok(url) => url,
        Err(err) => return error_response(500, err.to_string()),
    };

    HttpResponse::Ok().json(ParticipateResponse {
        lease_id,
        participant_id,
        step: next_step,
        expires_at,
        input_url,
        output_url,
        contribution_url,
    })
}

async fn submit(
    state: web::Data<AppState>,
    ceremony_id: web::Path<String>,
    body: web::Json<SubmitRequest>,
) -> HttpResponse {
    let ceremony_id = ceremony_id.into_inner();
    log::info!(
        "Processing submission for ceremony {} from participant {}",
        ceremony_id,
        body.participant_id
    );
    let now = unix_seconds();

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => return error_response(500, err.to_string()),
    };

    let lease_row = match sqlx::query(
        "SELECT ceremony_id, participant_id, status, input_key, output_key, contribution_key, step, expires_at
         FROM leases WHERE id = ?",
    )
    .bind(&body.lease_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(404, "lease not found".to_string()),
        Err(err) => return error_response(500, err.to_string()),
    };

    let lease_ceremony: String = lease_row.get("ceremony_id");
    if lease_ceremony != ceremony_id {
        return error_response(400, "lease ceremony mismatch".to_string());
    }
    let participant_id: String = lease_row.get("participant_id");
    if participant_id != body.participant_id {
        return error_response(400, "participant mismatch".to_string());
    }
    let status: String = lease_row.get("status");
    if status != "active" {
        return error_response(409, "lease is not active".to_string());
    }
    let expires_at: i64 = lease_row.get("expires_at");
    if expires_at as u64 <= now {
        let _ = sqlx::query("UPDATE leases SET status = ? WHERE id = ?")
            .bind("expired")
            .bind(&body.lease_id)
            .execute(&mut *tx)
            .await;
        let _ = tx.commit().await;
        return error_response(409, "lease expired".to_string());
    }

    let input_key: String = lease_row.get("input_key");
    let output_key: String = lease_row.get("output_key");
    let contribution_key: String = lease_row.get("contribution_key");
    let step: i64 = lease_row.get("step");

    let ceremony_row = match sqlx::query("SELECT circuit FROM ceremonies WHERE id = ?")
        .bind(&ceremony_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(404, "ceremony not found".to_string()),
        Err(err) => return error_response(500, err.to_string()),
    };

    let circuit_str: String = ceremony_row.get("circuit");
    let circuit = match CeremonyCircuit::parse(&circuit_str) {
        Ok(circuit) => circuit,
        Err(err) => return error_response(500, err.to_string()),
    };

    log::info!("Downloading output transcript from S3: {}...", output_key);
    let output_bytes = match state.s3.get_bytes(&output_key).await {
        Ok(bytes) => bytes,
        Err(err) => return error_response(500, err.to_string()),
    };
    log::info!(
        "Output transcript downloaded ({} bytes)",
        output_bytes.len()
    );

    log::info!("Deserializing output transcript...");
    let output_transcript = match Transcript::<Bn254>::deserialize_uncompressed(&output_bytes[..]) {
        Ok(transcript) => transcript,
        Err(err) => return error_response(400, err.to_string()),
    };
    log::info!("Output transcript deserialized");

    log::info!("Verifying transcript for circuit {:?}...", circuit.as_str());
    if let Err(err) = verify_transcript(
        &state.accum,
        circuit,
        &output_transcript,
        state.config.pedersen_seed,
    ) {
        return error_response(400, err.to_string());
    }
    log::info!("Transcript verification passed");

    log::info!("Downloading input transcript from S3: {}...", input_key);
    let input_bytes = match state.s3.get_bytes(&input_key).await {
        Ok(bytes) => bytes,
        Err(err) => return error_response(500, err.to_string()),
    };
    log::info!("Input transcript downloaded ({} bytes)", input_bytes.len());

    log::info!("Deserializing input transcript...");
    let input_transcript = match Transcript::<Bn254>::deserialize_uncompressed(&input_bytes[..]) {
        Ok(transcript) => transcript,
        Err(err) => return error_response(400, err.to_string()),
    };
    log::info!("Input transcript deserialized");

    log::info!("Verifying key transform...");
    if let Err(err) = verify_key_transform(&input_transcript, &output_transcript) {
        return error_response(400, err.to_string());
    }
    log::info!("Key transform verification passed");

    if let Err(err) = sqlx::query(
        "UPDATE ceremonies SET current_head_key = ?, step = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&output_key)
    .bind(step)
    .bind(now as i64)
    .bind(&ceremony_id)
    .execute(&mut *tx)
    .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = sqlx::query("UPDATE leases SET status = ? WHERE id = ?")
        .bind("completed")
        .bind(&body.lease_id)
        .execute(&mut *tx)
        .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = sqlx::query(
        "UPDATE contributions SET status = ?, updated_at = ? WHERE ceremony_id = ? AND step = ?",
    )
    .bind("completed")
    .bind(now as i64)
    .bind(&ceremony_id)
    .bind(step)
    .execute(&mut *tx)
    .await
    {
        return error_response(500, err.to_string());
    }

    if let Err(err) = tx.commit().await {
        return error_response(500, err.to_string());
    }

    let latest = LatestMetadata {
        step: step as u64,
        transcript_key: output_key.clone(),
        contribution_key: Some(contribution_key.clone()),
        updated_at: now,
    };
    let latest_bytes = match serde_json::to_vec(&latest) {
        Ok(bytes) => bytes,
        Err(err) => return error_response(500, err.to_string()),
    };

    log::info!("Updating latest metadata in S3...");
    if let Err(err) = state
        .s3
        .put_bytes(&latest_key(&ceremony_id), latest_bytes, "application/json")
        .await
    {
        return error_response(500, err.to_string());
    }

    log::info!(
        "Submission completed successfully for ceremony {} step {}",
        ceremony_id,
        step
    );
    HttpResponse::Ok().json(SubmitResponse {
        step: step as u64,
        transcript_key: output_key,
    })
}

async fn ensure_ceremony_absent(pool: &SqlitePool, ceremony_id: &str) -> Result<()> {
    let exists = sqlx::query("SELECT 1 FROM ceremonies WHERE id = ?")
        .bind(ceremony_id)
        .fetch_optional(pool)
        .await?
        .is_some();
    if exists {
        bail!("ceremony already exists")
    }
    Ok(())
}

async fn insert_ceremony(
    pool: &SqlitePool,
    ceremony_id: &str,
    circuit: CeremonyCircuit,
    current_head_key: &str,
    now: u64,
    config: &Config,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO ceremonies (id, circuit, current_head_key, step, lease_ttl_seconds, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ceremony_id)
    .bind(circuit.as_str())
    .bind(current_head_key)
    .bind(0i64)
    .bind(config.lease_ttl.as_secs() as i64)
    .bind(now as i64)
    .bind(now as i64)
    .execute(pool)
    .await?;
    Ok(())
}

async fn expire_active_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ceremony_id: &str,
    now: u64,
) -> Result<()> {
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
) -> Result<Option<u64>> {
    let row = sqlx::query(
        "SELECT expires_at FROM leases WHERE ceremony_id = ? AND status = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(ceremony_id)
    .bind("active")
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| row.get::<i64, _>("expires_at") as u64))
}

fn build_initial_transcript(
    accum: &Accumulator<Bn254>,
    circuit: CeremonyCircuit,
    pedersen_seed: u64,
) -> Result<Transcript<Bn254>> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let circuit = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::WithdrawGlobal => {
            let circuit = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderRoot => {
            let circuit = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let circuit =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let circuit = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            Transcript::new_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
    }
}

fn verify_transcript(
    accum: &Accumulator<Bn254>,
    circuit: CeremonyCircuit,
    transcript: &Transcript<Bn254>,
    pedersen_seed: u64,
) -> Result<()> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let circuit = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::WithdrawGlobal => {
            let circuit = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderRoot => {
            let circuit = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let circuit =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let circuit = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
    }
    Ok(())
}

fn verify_key_transform(prev: &Transcript<Bn254>, next: &Transcript<Bn254>) -> Result<()> {
    if next.contributions.len() != prev.contributions.len() + 1 {
        bail!("contribution chain length mismatch")
    }
    if next.contributions[..prev.contributions.len()] != prev.contributions {
        bail!("contribution chain mismatch")
    }

    let prev_partial = PartialKey::from(&prev.key);
    let next_partial = PartialKey::from(&next.key);
    let proof = next
        .contributions
        .last()
        .copied()
        .context("missing contribution proof")?;

    Transcript::<Bn254>::verify_key_transform(&prev_partial, &next_partial, &proof.proof)
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

fn build_withdraw_circuit<const DEPTH: usize>() -> Result<SingleWithdrawCircuit<Fr, DEPTH>> {
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    let zero = Fr::from(0u64);
    Ok(SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon2_params: poseidon2_config,
        poseidon3_params: poseidon3_config,
        merkle_root: Some(zero),
        recipient: Some(zero),
        withdraw_value: Some(zero),
        from: Some(zero),
        value: Some(zero),
        delta: Some(zero),
        secret: Some(zero),
        leaf_index: Some(0),
        siblings: [(); DEPTH].map(|_| Some(zero)),
    })
}

fn build_decider_circuit<C>(pedersen_seed: u64) -> Result<DeciderEthCircuit<G1, G2>>
where
    C: FCircuit<
        Fr,
        Params = (
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
        ),
    >,
    FParams<C>: Clone,
{
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let circuit = C::new(default_f_params::<C>()?).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, C>(&poseidon_config, circuit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    let mut rng = StdRng::seed_from_u64(pedersen_seed);
    let (cf_cs_pp, _) = Pedersen::<G2>::setup(&mut rng, pedersen_len)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(DeciderEthCircuit::<G1, G2>::dummy((
        r1cs,
        cf_r1cs,
        cf_cs_pp,
        poseidon_config,
        (),
        (),
        state_len,
        2,
    )))
}

fn default_f_params<C>() -> Result<(
    ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
    ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
)>
where
    C: FCircuit<
        Fr,
        Params = (
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
        ),
    >,
{
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    Ok((poseidon2_config, poseidon3_config))
}

fn load_accumulator(path: &PathBuf) -> Result<Accumulator<Bn254>> {
    Accumulator::<Bn254>::from_ptau_file(path)
        .with_context(|| format!("failed to load ptau from {}", path.display()))
}

fn transcript_key(ceremony_id: &str, step: u64) -> String {
    format!("ceremonies/{}/transcripts/{}.bin", ceremony_id, step)
}

fn contribution_key(ceremony_id: &str, step: u64) -> String {
    format!("ceremonies/{}/contributions/{}.bin", ceremony_id, step)
}

fn latest_key(ceremony_id: &str) -> String {
    format!("ceremonies/{}/latest.json", ceremony_id)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn error_response(status: u16, message: String) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .json(ErrorResponse { error: message })
}

fn default_ptau_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("zerc20")
            .join("ptau")
            .join("ppot_0080_24.ptau")
    } else {
        PathBuf::from("ptau").join("ppot_0080_24.ptau")
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
