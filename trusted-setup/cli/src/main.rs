use std::{
    borrow::Cow,
    cmp::max,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use alloy::signers::{local::PrivateKeySigner, SignerSync};
use anyhow::{bail, Context, Result};
use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_ec::pairing::Pairing;
use ark_grumpkin::Projective as G2;
use ark_poly_commit::kzg10::VerifierKey as KzgVerifierKey;
use ark_serialize::CanonicalDeserialize;
use arkworks_phase2::{
    accumulator::Accumulator,
    transcript::{ContributionContext, Transcript},
    utils::serialize_uncompressed,
};
use backoff::{future::retry, ExponentialBackoff};
use bytes::Bytes;
use clap::{Args, Parser, Subcommand, ValueEnum};
use folding_schemes::{
    arith::Arith,
    commitment::{
        kzg::{ProverKey as KzgProverKey, KZG},
        pedersen::Pedersen,
        CommitmentScheme,
    },
    folding::nova::{get_r1cs, PreprocessorParam},
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
    FoldingScheme,
};
use futures::{stream, StreamExt};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rand::{
    rngs::{OsRng, StdRng},
    SeedableRng,
};
use reqwest::{header::CONTENT_LENGTH, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url as ParsedUrl;

use trusted_setup_common::{
    build_initial_transcript_cached, default_ptau_path, initial_transcript_path, load_accumulator,
    ptau_path_for_circuit, ptau_url_for_power, CeremonyCircuit, LatestMetadata,
    SUPPORTED_PTAU_POWERS,
};
use zkp::groth16::params::Groth16Params;
use zkp::nova::{
    constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    params::{DeciderParams, FParams, NovaParams, N},
    root_nova::RootCircuit,
    withdraw_nova::WithdrawCircuit,
};
use zkp::utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config};

// ============================================================================
// Constants
// ============================================================================

const UPLOAD_CHUNK_SIZE: usize = 64 * 1024;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_READ_TIMEOUT_SECS: u64 = 1800; // 30 minutes (submit verification takes time)
const MAX_RETRY_ATTEMPTS: u32 = 3;
const PEDERSEN_SEED: u64 = 42;

/// Expected SHA256 hash of the default PTAU file (first 16 bytes for display)
const DEFAULT_PTAU_SIZE: u64 = 2_281_701_482; // ~2.1GB

// ============================================================================
// CLI Structure
// ============================================================================

#[derive(Parser, Debug)]
#[command(author, version, about = "trusted setup CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(subcommand)]
    Ptau(PtauCommand),
    Contribute(ContributeArgs),
    Finalize(FinalizeArgs),
    /// Check lease status
    Status(StatusArgs),
    /// Resume an interrupted contribution
    Resume(ResumeArgs),
    /// Generate initial transcript for a circuit (can be used to speed up coordinator initialization)
    GenerateInitialTranscript(GenerateInitialTranscriptArgs),
}

#[derive(Subcommand, Debug)]
enum PtauCommand {
    Download(PtauDownloadArgs),
    /// Verify PTAU file hash
    Verify(PtauVerifyArgs),
}

#[derive(Args, Debug)]
struct PtauDownloadArgs {
    /// Ptau URL to download. If not specified, uses URL based on --power.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_URL")]
    url: Option<String>,

    /// PTAU power (14 for groth16, 24 for decider). Determines URL and output path if not specified.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_POWER", default_value_t = 24)]
    power: u8,

    /// Output path for the ptau file. If not specified, uses default path based on --power.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Overwrite the existing file if present.
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Skip SHA256 verification after download.
    #[arg(long, default_value_t = false)]
    skip_verify: bool,
}

#[derive(Args, Debug)]
struct PtauVerifyArgs {
    /// Path to the PTAU file to verify.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    path: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
enum CliCeremonyCircuit {
    #[value(name = "withdraw_local")]
    WithdrawLocal,
    #[value(name = "withdraw_global")]
    WithdrawGlobal,
    #[value(name = "decider_root", alias = "root")]
    DeciderRoot,
    #[value(name = "decider_withdraw_local")]
    DeciderWithdrawLocal,
    #[value(name = "decider_withdraw_global")]
    DeciderWithdrawGlobal,
}

impl From<CliCeremonyCircuit> for CeremonyCircuit {
    fn from(c: CliCeremonyCircuit) -> Self {
        match c {
            CliCeremonyCircuit::WithdrawLocal => CeremonyCircuit::WithdrawLocal,
            CliCeremonyCircuit::WithdrawGlobal => CeremonyCircuit::WithdrawGlobal,
            CliCeremonyCircuit::DeciderRoot => CeremonyCircuit::DeciderRoot,
            CliCeremonyCircuit::DeciderWithdrawLocal => CeremonyCircuit::DeciderWithdrawLocal,
            CliCeremonyCircuit::DeciderWithdrawGlobal => CeremonyCircuit::DeciderWithdrawGlobal,
        }
    }
}

#[derive(Args, Debug)]
struct ContributeArgs {
    /// Coordinator base URL.
    #[arg(long, env = "TRUSTED_SETUP_COORDINATOR_URL")]
    coordinator_url: String,

    /// Ceremony identifier.
    #[arg(long, env = "TRUSTED_SETUP_CEREMONY_ID")]
    ceremony_id: String,

    /// Ethereum private key for signing contribution (required).
    /// Can be provided via CLI argument or TRUSTED_SETUP_ETH_PRIVATE_KEY env var.
    /// WARNING: CLI arguments may be visible in shell history. Prefer using the env var.
    #[arg(long, env = "TRUSTED_SETUP_ETH_PRIVATE_KEY")]
    eth_private_key: String,

    /// Optional seed for deterministic contribution.
    #[arg(long, env = "TRUSTED_SETUP_SEED")]
    seed: Option<String>,

    /// Connection timeout in seconds.
    #[arg(long, env = "TRUSTED_SETUP_CONNECT_TIMEOUT", default_value_t = DEFAULT_CONNECT_TIMEOUT_SECS)]
    connect_timeout: u64,

    /// Read timeout in seconds.
    #[arg(long, env = "TRUSTED_SETUP_READ_TIMEOUT", default_value_t = DEFAULT_READ_TIMEOUT_SECS)]
    read_timeout: u64,

    /// Save state file for resume capability.
    #[arg(long, env = "TRUSTED_SETUP_STATE_FILE")]
    state_file: Option<PathBuf>,

    /// Skip transcript verification (not recommended).
    #[arg(long, default_value_t = false)]
    skip_verify: bool,
}

#[derive(Args, Debug)]
struct FinalizeArgs {
    /// Coordinator base URL.
    #[arg(long, env = "TRUSTED_SETUP_COORDINATOR_URL")]
    coordinator_url: String,

    /// Ceremony id for the transcript.
    #[arg(long, env = "TRUSTED_SETUP_CEREMONY_ID")]
    ceremony_id: String,

    /// Ptau path (optional, auto-determined from circuit).
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    ptau_path: Option<PathBuf>,

    /// Output directory for artifacts.
    #[arg(long, env = "TRUSTED_SETUP_OUTPUT_DIR")]
    output_dir: Option<PathBuf>,

    /// Public base URL for latest.json and transcripts.
    #[arg(long, env = "TRUSTED_SETUP_PUBLIC_BASE_URL")]
    public_base_url: Option<String>,

    /// Connection timeout in seconds.
    #[arg(long, env = "TRUSTED_SETUP_CONNECT_TIMEOUT", default_value_t = DEFAULT_CONNECT_TIMEOUT_SECS)]
    connect_timeout: u64,

    /// Read timeout in seconds.
    #[arg(long, env = "TRUSTED_SETUP_READ_TIMEOUT", default_value_t = DEFAULT_READ_TIMEOUT_SECS)]
    read_timeout: u64,
}

#[derive(Args, Debug)]
struct StatusArgs {
    /// Coordinator base URL.
    #[arg(long, env = "TRUSTED_SETUP_COORDINATOR_URL")]
    coordinator_url: String,

    /// Ceremony identifier.
    #[arg(long, env = "TRUSTED_SETUP_CEREMONY_ID")]
    ceremony_id: String,

    /// Lease ID to check (optional, shows ceremony status if not provided).
    #[arg(long)]
    lease_id: Option<String>,
}

#[derive(Args, Debug)]
struct ResumeArgs {
    /// State file from a previous interrupted contribution.
    #[arg(long, env = "TRUSTED_SETUP_STATE_FILE")]
    state_file: PathBuf,

    /// Coordinator base URL.
    #[arg(long, env = "TRUSTED_SETUP_COORDINATOR_URL")]
    coordinator_url: String,
}

#[derive(Args, Debug)]
struct GenerateInitialTranscriptArgs {
    /// Circuit to generate initial transcript for.
    #[arg(long, env = "TRUSTED_SETUP_CIRCUIT", value_enum)]
    circuit: CliCeremonyCircuit,

    /// Ptau path.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    ptau_path: Option<PathBuf>,

    /// Output path for the initial transcript. If not specified, uses default path.
    #[arg(long)]
    output: Option<PathBuf>,

    /// PreparedAccumulator cache path. If not specified, uses default path.
    #[arg(long)]
    prepared_accum_cache: Option<PathBuf>,

    /// Overwrite the existing file if present.
    #[arg(long, default_value_t = false)]
    force: bool,
}

// ============================================================================
// API Types
// ============================================================================

#[derive(Deserialize)]
struct ParticipateResponse {
    lease_id: String,
    participant_id: String,
    circuit: String,
    step: u64,
    expires_at: u64,
    #[serde(default)]
    expires_in_seconds: u64,
    input_url: String,
    output_url: String,
    contribution_url: String,
}

#[derive(Serialize)]
struct SubmitRequest {
    lease_id: String,
    participant_id: String,
}

// Multipart upload types
#[derive(Serialize)]
struct MultipartStartRequest {
    lease_id: String,
    key_type: String, // "transcript" or "contribution"
}

#[derive(Deserialize)]
struct MultipartStartResponse {
    upload_id: String,
    key: String,
}

#[derive(Serialize)]
struct MultipartPresignRequest {
    lease_id: String,
    upload_id: String,
    key: String,
    part_number: i32,
}

#[derive(Deserialize)]
struct MultipartPresignResponse {
    url: String,
}

#[derive(Serialize, Clone)]
struct MultipartPartInfo {
    part_number: i32,
    etag: String,
}

#[derive(Serialize)]
struct MultipartCompleteRequest {
    lease_id: String,
    upload_id: String,
    key: String,
    parts: Vec<MultipartPartInfo>,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    #[allow(dead_code)]
    code: u16,
}

#[derive(Deserialize, Debug)]
struct CeremonyStatus {
    id: String,
    circuit: String,
    current_step: u64,
    has_active_lease: bool,
    active_lease_expires_at: Option<u64>,
    total_contributions: u64,
}

#[derive(Deserialize, Debug)]
struct LeaseStatus {
    id: String,
    status: String,
    step: u64,
    remaining_seconds: i64,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Debug)]
enum TranscriptSource {
    Url(ParsedUrl),
    Path(PathBuf),
}

/// State saved for resume capability.
#[derive(Serialize, Deserialize)]
struct ContributionState {
    ceremony_id: String,
    circuit: String,
    lease_id: String,
    participant_id: String,
    step: u64,
    expires_at: u64,
    output_url: String,
    contribution_url: String,
    transcript_uploaded: bool,
    contribution_uploaded: bool,
}

// ============================================================================
// HTTP Client Builder
// ============================================================================

fn build_http_client(connect_timeout: u64, read_timeout: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(connect_timeout))
        .read_timeout(Duration::from_secs(read_timeout))
        .pool_max_idle_per_host(2)
        .build()
        .context("failed to build HTTP client")
}

// ============================================================================
// Graceful Shutdown
// ============================================================================

fn setup_shutdown_handler() -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    ctrlc::set_handler(move || {
        if shutdown_clone.swap(true, Ordering::SeqCst) {
            // Second Ctrl+C, force exit
            eprintln!("\nForce exit requested");
            std::process::exit(1);
        }
        eprintln!("\nGraceful shutdown requested. Press Ctrl+C again to force exit.");
    })
    .expect("Error setting Ctrl-C handler");

    shutdown
}

fn check_shutdown(shutdown: &AtomicBool) -> Result<()> {
    if shutdown.load(Ordering::SeqCst) {
        bail!("Operation cancelled by user");
    }
    Ok(())
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let shutdown = setup_shutdown_handler();
    let cli = Cli::parse();

    match cli.command {
        Command::Ptau(PtauCommand::Download(args)) => download_ptau(args, &shutdown).await?,
        Command::Ptau(PtauCommand::Verify(args)) => verify_ptau(args).await?,
        Command::Contribute(args) => contribute(args, &shutdown).await?,
        Command::Finalize(args) => finalize(args, &shutdown).await?,
        Command::Status(args) => check_status(args).await?,
        Command::Resume(args) => resume_contribution(args, &shutdown).await?,
        Command::GenerateInitialTranscript(args) => generate_initial_transcript(args)?,
    }

    Ok(())
}

// ============================================================================
// PTAU Commands
// ============================================================================

async fn download_ptau(args: PtauDownloadArgs, shutdown: &AtomicBool) -> Result<()> {
    // Validate power
    if !SUPPORTED_PTAU_POWERS.contains(&args.power) {
        bail!(
            "unsupported ptau power {}. Supported powers: {:?}",
            args.power,
            SUPPORTED_PTAU_POWERS
        );
    }

    // Determine URL and output path based on power
    let url = args.url.unwrap_or_else(|| {
        ptau_url_for_power(args.power)
            .expect("validated power")
            .to_string()
    });

    let output = args
        .output
        .unwrap_or_else(|| trusted_setup_common::ptau_path_for_power(args.power));

    if output.exists() && !args.force {
        bail!(
            "ptau already exists at {} (use --force to overwrite)",
            output.display()
        );
    }

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let client = build_http_client(DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_READ_TIMEOUT_SECS)?;

    println!("Downloading PTAU (power {}) from {}...", args.power, url);
    let resp = retry_request(|| async {
        check_shutdown(shutdown)?;
        client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to download ptau from {}", url))
    })
    .await?;

    if !resp.status().is_success() {
        bail!("ptau download failed with status {}", resp.status());
    }

    let tmp_path = output.with_extension("part");
    let progress = build_progress("downloading ptau", resp.content_length());
    let mut hasher = Sha256::new();
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_context(|| format!("failed to create {}", tmp_path.display()))?;
    let mut stream = resp.bytes_stream();
    let mut total_bytes = 0u64;

    while let Some(chunk) = stream.next().await {
        check_shutdown(shutdown)?;
        let chunk = chunk.context("failed to read ptau chunk")?;
        hasher.update(&chunk);
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("failed to write ptau chunk")?;
        total_bytes += chunk.len() as u64;
        progress.inc(chunk.len() as u64);
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .context("failed to flush ptau output")?;
    drop(file);

    tokio::fs::rename(&tmp_path, &output)
        .await
        .with_context(|| format!("failed to move ptau to {}", output.display()))?;

    progress.finish_and_clear();
    println!(
        "PTAU (power {}) downloaded to {} ({} bytes)",
        args.power,
        output.display(),
        total_bytes
    );

    if !args.skip_verify {
        // Size verification is only for power 24 (we have the expected size)
        if args.power == 24 {
            println!("Verifying file size...");
            if total_bytes != DEFAULT_PTAU_SIZE {
                bail!(
                    "PTAU file size mismatch: expected {} bytes, got {} bytes",
                    DEFAULT_PTAU_SIZE,
                    total_bytes
                );
            }
            println!("File size verified successfully");
        }

        let hash = hex::encode(hasher.finalize());
        println!("SHA256: {}", hash);
    }

    Ok(())
}

async fn verify_ptau(args: PtauVerifyArgs) -> Result<()> {
    let path = args.path.unwrap_or_else(default_ptau_path);
    if !path.exists() {
        bail!("PTAU file not found at {}", path.display());
    }

    println!("Verifying PTAU file at {}...", path.display());

    let metadata = tokio::fs::metadata(&path).await?;
    let file_size = metadata.len();

    println!("File size: {} bytes", file_size);
    if file_size != DEFAULT_PTAU_SIZE {
        println!(
            "Warning: File size mismatch (expected {} bytes)",
            DEFAULT_PTAU_SIZE
        );
    } else {
        println!("File size matches expected value");
    }

    println!("Computing SHA256 hash...");
    let progress = build_progress("hashing", Some(file_size));

    let file = tokio::fs::File::open(&path).await?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

    loop {
        let n = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        progress.inc(n as u64);
    }

    progress.finish_and_clear();
    let hash = hex::encode(hasher.finalize());
    println!("SHA256: {}", hash);
    println!("Verification complete");

    Ok(())
}

// ============================================================================
// Generate Initial Transcript Command
// ============================================================================

fn generate_initial_transcript(args: GenerateInitialTranscriptArgs) -> Result<()> {
    let circuit: CeremonyCircuit = args.circuit.into();

    // Determine paths
    let ptau_path = args
        .ptau_path
        .unwrap_or_else(|| ptau_path_for_circuit(circuit));
    let output_path = args
        .output
        .unwrap_or_else(|| initial_transcript_path(circuit));

    // Check if output exists
    if output_path.exists() && !args.force {
        bail!(
            "Initial transcript already exists at {} (use --force to overwrite)",
            output_path.display()
        );
    }

    println!("=== Generate Initial Transcript ===");
    println!("Circuit: {}", circuit.as_str());
    println!("PTAU path: {}", ptau_path.display());
    println!("Output path: {}", output_path.display());
    if let Some(ref path) = args.prepared_accum_cache {
        println!("PreparedAccumulator cache (override): {}", path.display());
    } else {
        println!("PreparedAccumulator cache: (auto-computed based on ptau_power and domain_size)");
    }
    println!();

    // Generate initial transcript with caching (PTAU is loaded lazily only if cache is invalid/missing)
    println!("Generating initial transcript...");
    let transcript = build_initial_transcript_cached(
        &ptau_path,
        circuit,
        PEDERSEN_SEED,
        args.prepared_accum_cache.as_deref(),
    )?;
    println!(
        "Initial transcript generated ({} contributions)",
        transcript.contributions.len()
    );

    // Create output directory if needed
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // Serialize and save
    println!("Saving initial transcript to {}...", output_path.display());
    let bytes =
        serialize_uncompressed(&transcript).context("failed to serialize initial transcript")?;
    fs::write(&output_path, &bytes)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    println!("Initial transcript saved ({} bytes)", bytes.len());
    println!();
    println!("=== Done ===");

    Ok(())
}

// ============================================================================
// Contribute Command
// ============================================================================

async fn contribute(args: ContributeArgs, shutdown: &AtomicBool) -> Result<()> {
    let client = build_http_client(args.connect_timeout, args.read_timeout)?;
    let base_url = Url::parse(&args.coordinator_url)
        .with_context(|| format!("invalid coordinator url {}", args.coordinator_url))?;

    // Parse Ethereum private key and derive address
    let eth_private_key = args.eth_private_key.trim();
    let eth_private_key = eth_private_key
        .strip_prefix("0x")
        .unwrap_or(eth_private_key);
    let signer: PrivateKeySigner = eth_private_key
        .parse()
        .context("invalid Ethereum private key")?;
    let eth_address: [u8; 20] = signer.address().into_array();

    println!("Contributor address: 0x{}", hex::encode(eth_address));

    // Request participation
    println!(
        "Requesting participation in ceremony {}...",
        args.ceremony_id
    );
    let participate_url = base_url
        .join(&format!("/api/ceremonies/{}/participate", args.ceremony_id))
        .context("failed to build participate url")?;

    let participate_resp = retry_request(|| async {
        check_shutdown(shutdown)?;
        client
            .get(participate_url.clone())
            .send()
            .await
            .context("participate request failed")
    })
    .await?;

    let participate: ParticipateResponse = handle_response(participate_resp, "participate").await?;

    let circuit = CeremonyCircuit::parse(&participate.circuit)
        .with_context(|| format!("invalid circuit from server: {}", participate.circuit))?;

    println!(
        "Lease acquired: {} (circuit: {}, step {}, expires in {} seconds)",
        participate.lease_id, participate.circuit, participate.step, participate.expires_in_seconds
    );

    // Save state for potential resume
    if let Some(ref state_file) = args.state_file {
        let state = ContributionState {
            ceremony_id: args.ceremony_id.clone(),
            circuit: participate.circuit.clone(),
            lease_id: participate.lease_id.clone(),
            participant_id: participate.participant_id.clone(),
            step: participate.step,
            expires_at: participate.expires_at,
            output_url: participate.output_url.clone(),
            contribution_url: participate.contribution_url.clone(),
            transcript_uploaded: false,
            contribution_uploaded: false,
        };
        save_state(state_file, &state)?;
        println!("State saved to {}", state_file.display());
    }

    // Download input transcript
    check_shutdown(shutdown)?;
    println!("Downloading input transcript...");
    let input_bytes =
        download_bytes_with_progress(&client, &participate.input_url, "transcript", shutdown)
            .await?;

    // Deserialize and verify
    check_shutdown(shutdown)?;
    println!("Deserializing transcript...");
    let mut transcript = Transcript::<Bn254>::deserialize_uncompressed(&input_bytes[..])
        .context("failed to deserialize transcript")?;

    // Verify using initial transcript (optional but recommended)
    if !args.skip_verify {
        println!("Verifying transcript using initial transcript...");
        let initial_path = initial_transcript_path(circuit);
        if initial_path.exists() {
            let initial_transcript = load_initial_transcript_async(&initial_path).await?;
            transcript
                .verify_from_initial_transcript(&initial_transcript)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
                .context("transcript verification failed")?;
            println!("Transcript verified successfully");
        } else {
            bail!(
                "Initial transcript not found at {}.\n\
                 Generate it first with:\n  \
                 cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit {}\n\n\
                 Or use --skip-verify to skip verification (not recommended).",
                initial_path.display(),
                circuit.as_str()
            );
        }
    } else {
        println!("Skipping transcript verification (--skip-verify)");
    }

    // Contribute with Ethereum signature
    check_shutdown(shutdown)?;
    println!("Making contribution...");

    // Create contribution context for signature
    let context =
        ContributionContext::new(&args.ceremony_id, participate.step, &participate.circuit);

    // Create signing function
    let sign_fn = |message_hash: &[u8; 32]| -> Result<[u8; 65], arkworks_phase2::error::Error> {
        // Apply EIP-191 personal sign prefix
        let prefixed_hash = {
            let mut data = Vec::with_capacity(28 + 32);
            data.extend_from_slice(b"\x19Ethereum Signed Message:\n32");
            data.extend_from_slice(message_hash);
            alloy::primitives::keccak256(&data)
        };

        // Sign synchronously (PrivateKeySigner.sign_hash is sync)
        let signature = signer
            .sign_hash_sync(&prefixed_hash)
            .map_err(|e| arkworks_phase2::error::Error::Custom(format!("signing failed: {}", e)))?;

        // Convert to 65-byte format (r, s, v)
        let sig_bytes: [u8; 65] = signature.as_bytes().try_into().map_err(|_| {
            arkworks_phase2::error::Error::Custom("invalid signature length".to_string())
        })?;

        Ok(sig_bytes)
    };

    match &args.seed {
        Some(seed) => {
            transcript
                .contribute_seed(seed.as_bytes(), eth_address, sign_fn, &context)
                .context("failed to contribute using seed")?;
            println!("Contributed using provided seed");
        }
        None => {
            let mut rng = OsRng;
            transcript
                .contribute_rng(&mut rng, eth_address, sign_fn, &context)
                .context("failed to contribute using rng")?;
            println!("Contributed using random entropy");
        }
    }
    println!("Contribution signed by: 0x{}", hex::encode(eth_address));

    // Serialize
    check_shutdown(shutdown)?;
    println!("Serializing updated transcript...");
    let updated_bytes =
        serialize_uncompressed(&transcript).context("failed to serialize updated transcript")?;
    println!("Transcript serialized ({} bytes)", updated_bytes.len());

    let contribution = transcript
        .contributions
        .last()
        .context("missing contribution data")?;
    let contribution_bytes =
        serialize_uncompressed(contribution).context("failed to serialize contribution")?;

    // Upload transcript
    check_shutdown(shutdown)?;
    println!("Uploading updated transcript...");
    upload_bytes_smart(
        &client,
        &base_url,
        &args.ceremony_id,
        &participate.lease_id,
        "transcript",
        &participate.output_url,
        updated_bytes,
        "transcript",
        shutdown,
    )
    .await
    .context("failed to upload updated transcript")?;
    println!("Transcript uploaded");

    // Update state
    if let Some(ref state_file) = args.state_file {
        let mut state: ContributionState = load_state(state_file)?;
        state.transcript_uploaded = true;
        save_state(state_file, &state)?;
    }

    // Upload contribution
    check_shutdown(shutdown)?;
    println!("Uploading contribution proof...");
    upload_bytes_smart(
        &client,
        &base_url,
        &args.ceremony_id,
        &participate.lease_id,
        "contribution",
        &participate.contribution_url,
        contribution_bytes,
        "contribution",
        shutdown,
    )
    .await
    .context("failed to upload contribution")?;
    println!("Contribution uploaded");

    // Update state
    if let Some(ref state_file) = args.state_file {
        let mut state: ContributionState = load_state(state_file)?;
        state.contribution_uploaded = true;
        save_state(state_file, &state)?;
    }

    // Submit
    check_shutdown(shutdown)?;
    println!("Submitting to coordinator...");
    let submit_url = base_url
        .join(&format!("/api/ceremonies/{}/submit", args.ceremony_id))
        .context("failed to build submit url")?;

    let submit_resp = retry_request(|| async {
        check_shutdown(shutdown)?;
        client
            .post(submit_url.clone())
            .json(&SubmitRequest {
                lease_id: participate.lease_id.clone(),
                participant_id: participate.participant_id.clone(),
            })
            .send()
            .await
            .context("submit request failed")
    })
    .await?;

    let status_code = submit_resp.status();
    if status_code == reqwest::StatusCode::ACCEPTED {
        // 202 Accepted - processing in background, poll for completion
        println!("Submission accepted, waiting for verification to complete...");
        poll_submission_status(
            &client,
            &base_url,
            &args.ceremony_id,
            &participate.lease_id,
            shutdown,
        )
        .await?;
    } else if status_code.is_success() {
        // Direct success (backwards compatibility)
        let _: serde_json::Value = submit_resp
            .json()
            .await
            .context("failed to parse submit response")?;
    } else {
        // Error
        handle_response::<serde_json::Value>(submit_resp, "submit").await?;
    }

    // Clean up state file on success
    if let Some(ref state_file) = args.state_file {
        let _ = tokio::fs::remove_file(state_file).await;
    }

    println!(
        "Contribution submitted successfully for step {}",
        participate.step
    );
    Ok(())
}

/// Poll lease status until submission is completed or failed.
async fn poll_submission_status(
    client: &reqwest::Client,
    base_url: &Url,
    ceremony_id: &str,
    lease_id: &str,
    shutdown: &AtomicBool,
) -> Result<()> {
    let lease_url = base_url
        .join(&format!(
            "/api/ceremonies/{}/leases/{}",
            ceremony_id, lease_id
        ))
        .context("failed to build lease url")?;

    let poll_interval = Duration::from_secs(10);
    let progress = build_progress("verifying submission", None);

    loop {
        check_shutdown(shutdown)?;

        let resp = client
            .get(lease_url.clone())
            .send()
            .await
            .context("failed to check lease status")?;

        // Handle rate limiting
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            progress.set_message("rate limited, waiting...");
            tokio::time::sleep(Duration::from_secs(30)).await;
            continue;
        }

        let status: LeaseStatus = handle_response(resp, "lease status").await?;

        match status.status.as_str() {
            "completed" => {
                progress.finish_and_clear();
                println!("Verification completed successfully");
                return Ok(());
            }
            "failed" => {
                progress.finish_and_clear();
                let error_msg = status
                    .error_message
                    .unwrap_or_else(|| "unknown error".to_string());
                bail!("Submission verification failed: {}", error_msg);
            }
            "processing" => {
                progress.set_message(format!("verifying submission (step {})", status.step));
                tokio::time::sleep(poll_interval).await;
            }
            other => {
                progress.finish_and_clear();
                bail!("Unexpected lease status: {}", other);
            }
        }
    }
}

// ============================================================================
// Resume Command
// ============================================================================

async fn resume_contribution(args: ResumeArgs, _shutdown: &AtomicBool) -> Result<()> {
    let state: ContributionState = load_state(&args.state_file)?;
    println!("Resuming contribution for ceremony {}", state.ceremony_id);

    let client = build_http_client(DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_READ_TIMEOUT_SECS)?;
    let base_url = Url::parse(&args.coordinator_url)?;

    // Check if lease is still valid
    let lease_url = base_url.join(&format!(
        "/api/ceremonies/{}/leases/{}",
        state.ceremony_id, state.lease_id
    ))?;

    let lease_resp = client.get(lease_url).send().await?;
    let lease_status: LeaseStatus = handle_response(lease_resp, "lease status").await?;

    if lease_status.status != "active" {
        bail!(
            "Lease is no longer active (status: {})",
            lease_status.status
        );
    }

    if lease_status.remaining_seconds <= 0 {
        bail!("Lease has expired");
    }

    println!(
        "Lease still valid ({} seconds remaining)",
        lease_status.remaining_seconds
    );

    // If both uploaded, just submit
    if state.transcript_uploaded && state.contribution_uploaded {
        println!("Both files already uploaded, submitting...");
        let submit_url = base_url.join(&format!("/api/ceremonies/{}/submit", state.ceremony_id))?;

        let submit_resp = client
            .post(submit_url)
            .json(&SubmitRequest {
                lease_id: state.lease_id.clone(),
                participant_id: state.participant_id.clone(),
            })
            .send()
            .await?;

        handle_response::<serde_json::Value>(submit_resp, "submit").await?;
        let _ = tokio::fs::remove_file(&args.state_file).await;
        println!("Contribution submitted successfully");
        return Ok(());
    }

    println!("Resume capability requires re-running contribution from the beginning.");
    println!("Please re-run the contribute command.");
    bail!("Partial resume not supported - please re-run contribute command");
}

// ============================================================================
// Status Command
// ============================================================================

async fn check_status(args: StatusArgs) -> Result<()> {
    let client = build_http_client(DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_READ_TIMEOUT_SECS)?;
    let base_url = Url::parse(&args.coordinator_url)?;

    if let Some(lease_id) = args.lease_id {
        // Check specific lease
        let url = base_url.join(&format!(
            "/api/ceremonies/{}/leases/{}",
            args.ceremony_id, lease_id
        ))?;
        let resp = client.get(url).send().await?;
        let status: LeaseStatus = handle_response(resp, "lease status").await?;

        println!("Lease Status:");
        println!("  ID: {}", status.id);
        println!("  Status: {}", status.status);
        println!("  Step: {}", status.step);
        println!("  Remaining: {} seconds", status.remaining_seconds);
    } else {
        // Check ceremony status
        let url = base_url.join(&format!("/api/ceremonies/{}", args.ceremony_id))?;
        let resp = client.get(url).send().await?;
        let status: CeremonyStatus = handle_response(resp, "ceremony status").await?;

        println!("Ceremony Status:");
        println!("  ID: {}", status.id);
        println!("  Circuit: {}", status.circuit);
        println!("  Current Step: {}", status.current_step);
        println!("  Total Contributions: {}", status.total_contributions);
        println!("  Has Active Lease: {}", status.has_active_lease);
        if let Some(expires) = status.active_lease_expires_at {
            println!("  Active Lease Expires At: {}", expires);
        }
    }

    Ok(())
}

// ============================================================================
// Finalize Command
// ============================================================================

async fn finalize(args: FinalizeArgs, shutdown: &AtomicBool) -> Result<()> {
    let client = build_http_client(args.connect_timeout, args.read_timeout)?;
    let base_url = Url::parse(&args.coordinator_url)
        .with_context(|| format!("invalid coordinator url {}", args.coordinator_url))?;

    // Get ceremony info from coordinator
    println!("Fetching ceremony info from coordinator...");
    let ceremony_url = base_url
        .join(&format!("/api/ceremonies/{}", args.ceremony_id))
        .context("failed to build ceremony url")?;

    let ceremony_resp = client
        .get(ceremony_url)
        .send()
        .await
        .context("failed to fetch ceremony info")?;

    let ceremony: CeremonyStatus = handle_response(ceremony_resp, "ceremony info").await?;
    let circuit = CeremonyCircuit::parse(&ceremony.circuit)
        .with_context(|| format!("invalid circuit from server: {}", ceremony.circuit))?;

    println!(
        "Ceremony: {} (circuit: {}, step: {}, contributions: {})",
        ceremony.id, ceremony.circuit, ceremony.current_step, ceremony.total_contributions
    );

    let ptau_path = args
        .ptau_path
        .unwrap_or_else(|| ptau_path_for_circuit(circuit));
    let output_dir = args
        .output_dir
        .unwrap_or_else(|| workspace_root().join("nova_artifacts"));
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    println!(
        "Finalizing circuit: {} -> {}",
        circuit.as_str(),
        output_dir.display()
    );

    // Load PTAU (only needed for decider circuits)
    println!("Loading PTAU from {}...", ptau_path.display());
    let accum = load_accumulator(&ptau_path)?;
    println!("PTAU loaded (g1: {} powers)", accum.tau_powers_g1.len());

    // Resolve transcript from coordinator
    println!("Resolving transcript...");
    let transcript_source = resolve_transcript_source(
        &client,
        None,
        Some(&args.ceremony_id),
        args.public_base_url.as_deref(),
        shutdown,
    )
    .await?;
    let transcript = load_transcript(&client, transcript_source, shutdown).await?;
    println!(
        "Transcript loaded ({} contributions)",
        transcript.contributions.len()
    );

    // Verify transcript using initial transcript (with signature verification)
    println!("Verifying transcript using initial transcript...");
    let initial_path = initial_transcript_path(circuit);
    if initial_path.exists() {
        let initial_transcript = load_initial_transcript_async(&initial_path).await?;
        transcript
            .verify_from_initial_transcript_with_context(
                &initial_transcript,
                &args.ceremony_id,
                &ceremony.circuit,
            )
            .map_err(|e| anyhow::anyhow!(e.to_string()))
            .context("transcript verification failed (including signatures)")?;
        println!("Transcript verified successfully (including all signatures)");
    } else {
        bail!(
            "Initial transcript not found at {}. Generate it first with:\n  \
             cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit {}",
            initial_path.display(),
            circuit.as_str()
        );
    }

    // Display contributor addresses
    println!();
    println!("=== Contributors ===");
    for (i, addr) in transcript.contributor_addresses().iter().enumerate() {
        println!("  Step {}: 0x{}", i + 1, hex::encode(addr));
    }
    println!("====================");
    println!();

    check_shutdown(shutdown)?;

    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            finalize_withdraw_groth16::<TRANSFER_TREE_HEIGHT>(
                "withdraw_local",
                &output_dir,
                transcript,
            )?;
        }
        CeremonyCircuit::WithdrawGlobal => {
            finalize_withdraw_groth16::<GLOBAL_TRANSFER_TREE_HEIGHT>(
                "withdraw_global",
                &output_dir,
                transcript,
            )?;
        }
        CeremonyCircuit::DeciderRoot => {
            finalize_decider::<RootCircuit<Fr>>(
                "root",
                &accum,
                &output_dir,
                transcript,
                PEDERSEN_SEED,
            )?;
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            finalize_decider::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
                "withdraw_local",
                &accum,
                &output_dir,
                transcript,
                PEDERSEN_SEED,
            )?;
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            finalize_decider::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                "withdraw_global",
                &accum,
                &output_dir,
                transcript,
                PEDERSEN_SEED,
            )?;
        }
    }

    Ok(())
}

// ============================================================================
// Finalization Functions
// ============================================================================

fn finalize_withdraw_groth16<const DEPTH: usize>(
    prefix: &str,
    output_dir: &Path,
    transcript: Transcript<Bn254>,
) -> Result<()> {
    println!("Extracting groth16 params...");
    let groth16_params = groth16_from_transcript(transcript);

    println!("Writing artifacts...");
    emit_groth16_artifacts(prefix, output_dir, &groth16_params)?;

    println!("Done: {prefix} groth16 params finalized");
    Ok(())
}

fn finalize_decider<C>(
    prefix: &str,
    accum: &Accumulator<Bn254>,
    output_dir: &Path,
    transcript: Transcript<Bn254>,
    pedersen_seed: u64,
) -> Result<()>
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
    let f_params = (poseidon2_config, poseidon3_config);

    println!("Building nova bundle (this may take a while)...");
    let nova_bundle = build_nova_bundle::<C>(accum, f_params, pedersen_seed)?;
    println!(
        "Nova bundle built (r1cs: {} constraints, state_len: {})",
        nova_bundle.r1cs.n_constraints(),
        nova_bundle.state_len
    );

    println!("Extracting groth16 params...");
    let groth16_params = groth16_from_transcript(transcript);

    println!("Writing artifacts...");
    emit_nova_artifacts(
        prefix,
        output_dir,
        &nova_bundle.params,
        &groth16_params,
        nova_bundle.state_len,
    )?;

    println!("Done: {prefix} nova and decider params finalized");
    Ok(())
}

fn groth16_from_transcript(transcript: Transcript<Bn254>) -> Groth16Params {
    let pk = transcript.key.key;
    let vk = pk.vk.clone();
    Groth16Params { pk, vk }
}

struct NovaBundle<C: FCircuit<Fr>>
where
    FParams<C>: Clone,
{
    params: NovaParams<C>,
    r1cs: folding_schemes::arith::r1cs::R1CS<Fr>,
    state_len: usize,
}

fn build_nova_bundle<C>(
    accum: &Accumulator<Bn254>,
    f_params: FParams<C>,
    pedersen_seed: u64,
) -> Result<NovaBundle<C>>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let circuit = C::new(f_params.clone()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, C>(&poseidon_config, circuit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let kzg_len = max(r1cs.n_constraints(), r1cs.n_witnesses());
    let (cs_pp, cs_vp) = kzg_params_from_ptau(accum, kzg_len)?;

    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    let mut rng = StdRng::seed_from_u64(pedersen_seed);
    let (cf_cs_pp, cf_cs_vp) = Pedersen::<G2>::setup(&mut rng, pedersen_len)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let circuit = C::new(f_params.clone()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let mut prep = PreprocessorParam::<G1, G2, C, KZG<'static, Bn254>, Pedersen<G2>, false>::new(
        poseidon_config.clone(),
        circuit,
    );
    prep.cs_pp = Some(cs_pp);
    prep.cs_vp = Some(cs_vp);
    prep.cf_cs_pp = Some(cf_cs_pp.clone());
    prep.cf_cs_vp = Some(cf_cs_vp);

    let mut rng = StdRng::seed_from_u64(0);
    let (pp, vp) =
        N::<C>::preprocess(&mut rng, &prep).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let params = NovaParams { f_params, pp, vp };

    Ok(NovaBundle {
        params,
        r1cs,
        state_len,
    })
}

fn kzg_params_from_ptau(
    accum: &Accumulator<Bn254>,
    len: usize,
) -> Result<(KzgProverKey<'static, G1>, KzgVerifierKey<Bn254>)> {
    let needed = len + 1;
    if accum.tau_powers_g1.len() < needed {
        bail!(
            "ptau g1 powers too short: need {}, have {}",
            needed,
            accum.tau_powers_g1.len()
        );
    }
    if accum.tau_powers_g2.len() < 2 {
        bail!("ptau g2 powers too short: need at least 2");
    }
    if accum.alpha_tau_powers_g1.is_empty() {
        bail!("ptau alpha powers missing");
    }

    let powers_of_g = accum.tau_powers_g1[..needed].to_vec();
    let g = accum.tau_powers_g1[0];
    let gamma_g = accum.alpha_tau_powers_g1[0];
    let h = accum.tau_powers_g2[0];
    let beta_h = accum.tau_powers_g2[1];

    let prepared_h = <Bn254 as Pairing>::G2Prepared::from(h);
    let prepared_beta_h = <Bn254 as Pairing>::G2Prepared::from(beta_h);

    let vk = KzgVerifierKey {
        g,
        gamma_g,
        h,
        beta_h,
        prepared_h,
        prepared_beta_h,
    };

    let pp = KzgProverKey {
        powers_of_g: Cow::Owned(powers_of_g),
    };

    Ok((pp, vk))
}

// ============================================================================
// Artifact Generation
// ============================================================================

fn emit_nova_artifacts<C>(
    prefix: &str,
    output_dir: &Path,
    nova_params: &NovaParams<C>,
    groth16_params: &Groth16Params,
    state_len: usize,
) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let (nova_pp_bytes, nova_vp_bytes) = nova_params
        .to_bytes()
        .context("failed to serialize nova params")?;

    let decider_pp = (groth16_params.pk.clone(), nova_params.pp.cs_pp.clone());
    let decider_vp = folding_schemes::folding::nova::decider_eth::VerifierParam {
        pp_hash: nova_params
            .vp
            .pp_hash()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?,
        snark_vp: groth16_params.vk.clone(),
        cs_vp: nova_params.vp.cs_vp.clone(),
    };
    let decider_params = DeciderParams::<C> {
        pp: decider_pp,
        vp: decider_vp,
    };

    let (decider_pp_bytes, decider_vp_bytes) = decider_params
        .to_bytes()
        .context("failed to serialize decider params")?;

    let pascal_case_prefix = to_pascal_case(prefix);
    let contract_name = format!("{}NovaDecider", pascal_case_prefix);
    let solidity = decider_params
        .verifier_solidity_code(state_len)
        .replace("NovaDecider", &contract_name);

    write_bytes(
        &output_dir.join(format!("{prefix}_nova_pp.bin")),
        &nova_pp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_nova_vp.bin")),
        &nova_vp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_decider_pp.bin")),
        &decider_pp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_decider_vp.bin")),
        &decider_vp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{pascal_case_prefix}NovaDecider.sol")),
        solidity.as_bytes(),
    )?;

    Ok(())
}

fn emit_groth16_artifacts(
    prefix: &str,
    output_dir: &Path,
    groth16_params: &Groth16Params,
) -> Result<()> {
    let (pk_bytes, vk_bytes) = groth16_params
        .to_bytes()
        .context("failed to serialize groth16 params")?;

    let pascal_case_prefix = to_pascal_case(prefix);
    let contract_name = format!("{}Groth16Verifier", pascal_case_prefix);
    let solidity = groth16_params
        .verifier_solidity_code()
        .context("failed to render groth16 solidity")?
        .replace("Groth16Verifier", &contract_name);

    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_pk.bin")),
        &pk_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_vk.bin")),
        &vk_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{pascal_case_prefix}Groth16Verifier.sol")),
        solidity.as_bytes(),
    )?;

    Ok(())
}

// ============================================================================
// HTTP Helpers
// ============================================================================

async fn handle_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
    operation: &str,
) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();

        // Try to parse as error response
        if let Ok(error_resp) = serde_json::from_str::<ErrorResponse>(&body) {
            bail!(
                "{} failed with status {}: {}",
                operation,
                status,
                error_resp.error
            );
        }

        bail!("{} failed with status {}: {}", operation, status, body);
    }

    resp.json()
        .await
        .with_context(|| format!("failed to parse {} response", operation))
}

async fn retry_request<F, Fut, T>(operation: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let backoff = ExponentialBackoff {
        max_elapsed_time: Some(Duration::from_secs(60)),
        max_interval: Duration::from_secs(10),
        ..Default::default()
    };

    retry(backoff, || async {
        operation().await.map_err(|e| {
            // Check if it's a retryable error
            let err_str = e.to_string();
            if err_str.contains("connection")
                || err_str.contains("timeout")
                || err_str.contains("temporarily")
            {
                backoff::Error::transient(e)
            } else {
                backoff::Error::permanent(e)
            }
        })
    })
    .await
}

async fn download_bytes_with_progress(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    shutdown: &AtomicBool,
) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to download {}", label))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("{} download failed with status {}: {}", label, status, body);
    }

    let total = resp.content_length();
    let progress = build_progress(&format!("downloading {}", label), total);
    let mut stream = resp.bytes_stream();
    let mut bytes = match total.and_then(|len| usize::try_from(len).ok()) {
        Some(capacity) => Vec::with_capacity(capacity),
        None => Vec::new(),
    };

    while let Some(chunk) = stream.next().await {
        check_shutdown(shutdown)?;
        let chunk = chunk.with_context(|| format!("failed to read {} bytes", label))?;
        bytes.extend_from_slice(&chunk);
        progress.inc(chunk.len() as u64);
    }
    progress.finish_and_clear();
    Ok(bytes)
}

async fn upload_bytes_with_retry(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    label: &str,
    shutdown: &AtomicBool,
) -> Result<()> {
    let total = body.len() as u64;
    let progress = build_progress(&format!("uploading {}", label), Some(total));

    let mut attempts = 0;
    loop {
        check_shutdown(shutdown)?;
        attempts += 1;

        let body_clone = body.clone();
        let progress_handle = progress.clone();

        let stream = stream::unfold(
            (body_clone, 0usize, progress_handle),
            |(body, offset, progress)| async move {
                if offset >= body.len() {
                    None
                } else {
                    let end = std::cmp::min(offset + UPLOAD_CHUNK_SIZE, body.len());
                    let chunk = Bytes::copy_from_slice(&body[offset..end]);
                    progress.inc((end - offset) as u64);
                    Some((Ok::<Bytes, std::io::Error>(chunk), (body, end, progress)))
                }
            },
        );

        let resp = client
            .put(url)
            .header(CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                progress.finish_and_clear();
                return Ok(());
            }
            Ok(r) => {
                let status = r.status();
                if attempts >= MAX_RETRY_ATTEMPTS || !is_retryable_status(status) {
                    let body = r.text().await.unwrap_or_default();
                    progress.finish_and_clear();
                    bail!("upload failed with status {}: {}", status, body);
                }
                progress.set_position(0);
                tokio::time::sleep(Duration::from_secs(attempts as u64)).await;
            }
            Err(e) => {
                if attempts >= MAX_RETRY_ATTEMPTS {
                    progress.finish_and_clear();
                    return Err(e.into());
                }
                progress.set_position(0);
                tokio::time::sleep(Duration::from_secs(attempts as u64)).await;
            }
        }
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status == StatusCode::GATEWAY_TIMEOUT
        || status.is_server_error()
}

// ============================================================================
// Multipart Upload
// ============================================================================

/// Threshold for using multipart upload (100MB)
const MULTIPART_THRESHOLD: usize = 100 * 1024 * 1024;
/// Part size for multipart upload (1GB)
const MULTIPART_PART_SIZE: usize = 1024 * 1024 * 1024;

/// Upload bytes using multipart upload via Coordinator API.
async fn upload_multipart(
    client: &reqwest::Client,
    base_url: &Url,
    ceremony_id: &str,
    lease_id: &str,
    key_type: &str, // "transcript" or "contribution"
    body: Vec<u8>,
    label: &str,
    shutdown: &AtomicBool,
) -> Result<()> {
    let total_size = body.len();
    let num_parts = (total_size + MULTIPART_PART_SIZE - 1) / MULTIPART_PART_SIZE;

    println!(
        "Using multipart upload for {} ({} bytes, {} parts)",
        label, total_size, num_parts
    );

    // Start multipart upload (with retry for rate limiting)
    let start_url = base_url
        .join(&format!("/api/ceremonies/{}/multipart/start", ceremony_id))
        .context("failed to build multipart start url")?;

    let start_data: MultipartStartResponse = {
        let mut start_attempts: u32 = 0;
        loop {
            start_attempts += 1;
            let start_resp = client
                .post(start_url.clone())
                .json(&MultipartStartRequest {
                    lease_id: lease_id.to_string(),
                    key_type: key_type.to_string(),
                })
                .send()
                .await
                .context("failed to start multipart upload")?;

            let status = start_resp.status();
            if status.is_success() {
                break start_resp
                    .json()
                    .await
                    .context("failed to parse multipart start response")?;
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait_secs = std::cmp::min(30, 5 * start_attempts as u64);
                eprintln!(
                    "Rate limited on multipart start, waiting {}s before retry...",
                    wait_secs
                );
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                continue;
            } else if start_attempts < MAX_RETRY_ATTEMPTS && is_retryable_status(status) {
                tokio::time::sleep(Duration::from_secs(start_attempts as u64)).await;
                continue;
            } else {
                let body = start_resp.text().await.unwrap_or_default();
                bail!("multipart start failed with status {}: {}", status, body);
            }
        }
    };

    let upload_id = start_data.upload_id;
    let key = start_data.key;

    let progress = build_progress(
        &format!("uploading {} (multipart)", label),
        Some(total_size as u64),
    );
    let mut completed_parts: Vec<MultipartPartInfo> = Vec::with_capacity(num_parts);

    // Upload each part
    for part_number in 1..=num_parts {
        check_shutdown(shutdown)?;

        let start = (part_number - 1) * MULTIPART_PART_SIZE;
        let end = std::cmp::min(start + MULTIPART_PART_SIZE, total_size);
        let part_bytes = body[start..end].to_vec();
        let part_size = part_bytes.len();

        // Get presigned URL for this part (with retry for rate limiting)
        let presign_url = base_url
            .join(&format!(
                "/api/ceremonies/{}/multipart/presign",
                ceremony_id
            ))
            .context("failed to build multipart presign url")?;

        let presign_data: MultipartPresignResponse = {
            let mut presign_attempts: u32 = 0;
            loop {
                presign_attempts += 1;
                let presign_resp = client
                    .post(presign_url.clone())
                    .json(&MultipartPresignRequest {
                        lease_id: lease_id.to_string(),
                        upload_id: upload_id.clone(),
                        key: key.clone(),
                        part_number: part_number as i32,
                    })
                    .send()
                    .await
                    .context("failed to get presigned URL for part")?;

                let status = presign_resp.status();
                if status.is_success() {
                    break presign_resp
                        .json()
                        .await
                        .context("failed to parse multipart presign response")?;
                } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    // Rate limited - wait and retry
                    let wait_secs = std::cmp::min(30, 5 * presign_attempts as u64);
                    eprintln!(
                        "Rate limited on presign for part {}, waiting {}s before retry...",
                        part_number, wait_secs
                    );
                    tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    continue;
                } else if presign_attempts < MAX_RETRY_ATTEMPTS && is_retryable_status(status) {
                    tokio::time::sleep(Duration::from_secs(presign_attempts as u64)).await;
                    continue;
                } else {
                    let body = presign_resp.text().await.unwrap_or_default();
                    bail!(
                        "multipart presign failed for part {} with status {}: {}",
                        part_number,
                        status,
                        body
                    );
                }
            }
        };

        // Upload part to presigned URL
        let mut attempts = 0;
        let etag = loop {
            attempts += 1;
            let upload_resp = client
                .put(&presign_data.url)
                .header(CONTENT_LENGTH, part_size)
                .body(part_bytes.clone())
                .send()
                .await;

            match upload_resp {
                Ok(r) if r.status().is_success() => {
                    // Get ETag from response header
                    let etag = r
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                        .context("missing etag in upload response")?;
                    break etag;
                }
                Ok(r) => {
                    let status = r.status();
                    if attempts >= MAX_RETRY_ATTEMPTS || !is_retryable_status(status) {
                        let body = r.text().await.unwrap_or_default();
                        bail!(
                            "part {} upload failed with status {}: {}",
                            part_number,
                            status,
                            body
                        );
                    }
                    tokio::time::sleep(Duration::from_secs(attempts as u64)).await;
                }
                Err(e) => {
                    if attempts >= MAX_RETRY_ATTEMPTS {
                        return Err(e).context(format!("failed to upload part {}", part_number));
                    }
                    tokio::time::sleep(Duration::from_secs(attempts as u64)).await;
                }
            }
        };

        completed_parts.push(MultipartPartInfo {
            part_number: part_number as i32,
            etag,
        });

        progress.inc(part_size as u64);
    }

    progress.finish_and_clear();

    // Complete multipart upload (with retry for rate limiting)
    let complete_url = base_url
        .join(&format!(
            "/api/ceremonies/{}/multipart/complete",
            ceremony_id
        ))
        .context("failed to build multipart complete url")?;

    let mut complete_attempts: u32 = 0;
    loop {
        complete_attempts += 1;
        let complete_resp = client
            .post(complete_url.clone())
            .json(&MultipartCompleteRequest {
                lease_id: lease_id.to_string(),
                upload_id: upload_id.clone(),
                key: key.clone(),
                parts: completed_parts.clone(),
            })
            .send()
            .await
            .context("failed to complete multipart upload")?;

        let status = complete_resp.status();
        if status.is_success() {
            break;
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let wait_secs = std::cmp::min(30, 5 * complete_attempts as u64);
            eprintln!(
                "Rate limited on multipart complete, waiting {}s before retry...",
                wait_secs
            );
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
            continue;
        } else if complete_attempts < MAX_RETRY_ATTEMPTS && is_retryable_status(status) {
            tokio::time::sleep(Duration::from_secs(complete_attempts as u64)).await;
            continue;
        } else {
            let body = complete_resp.text().await.unwrap_or_default();
            bail!("multipart complete failed with status {}: {}", status, body);
        }
    }

    Ok(())
}

/// Upload bytes using simple PUT or multipart upload depending on size.
async fn upload_bytes_smart(
    client: &reqwest::Client,
    base_url: &Url,
    ceremony_id: &str,
    lease_id: &str,
    key_type: &str,
    presigned_url: &str,
    body: Vec<u8>,
    label: &str,
    shutdown: &AtomicBool,
) -> Result<()> {
    if body.len() < MULTIPART_THRESHOLD {
        // Use simple PUT for small files
        upload_bytes_with_retry(client, presigned_url, body, label, shutdown).await
    } else {
        // Use multipart upload for large files
        upload_multipart(
            client,
            base_url,
            ceremony_id,
            lease_id,
            key_type,
            body,
            label,
            shutdown,
        )
        .await
    }
}

// ============================================================================
// Transcript Loading
// ============================================================================

async fn resolve_transcript_source(
    client: &reqwest::Client,
    explicit: Option<String>,
    ceremony_id: Option<&str>,
    public_base_url: Option<&str>,
    shutdown: &AtomicBool,
) -> Result<TranscriptSource> {
    if let Some(value) = explicit.filter(|s| !s.is_empty()) {
        return parse_transcript_source(&value);
    }

    let ceremony_id =
        ceremony_id.context("ceremony id is required when transcript path is not provided")?;
    let public_base_url = public_base_url
        .context("public base url is required when transcript path is not provided")?;

    let latest_url = build_latest_url(public_base_url, ceremony_id)?;

    check_shutdown(shutdown)?;
    let latest_resp = client
        .get(latest_url.as_str())
        .send()
        .await
        .context("failed to fetch latest metadata")?;

    if !latest_resp.status().is_success() {
        let status = latest_resp.status();
        let body = latest_resp.text().await.unwrap_or_default();
        bail!(
            "latest metadata fetch failed with status {}: {}",
            status,
            body
        );
    }

    let latest_bytes = latest_resp
        .bytes()
        .await
        .context("failed to read latest metadata")?;

    let latest: LatestMetadata =
        serde_json::from_slice(&latest_bytes).context("failed to parse latest metadata")?;
    let transcript_url = build_object_url(public_base_url, &latest.transcript_key)?;
    Ok(TranscriptSource::Url(transcript_url))
}

fn parse_transcript_source(value: &str) -> Result<TranscriptSource> {
    if value.starts_with("http://") || value.starts_with("https://") {
        let url = ParsedUrl::parse(value).context("invalid transcript url")?;
        Ok(TranscriptSource::Url(url))
    } else {
        Ok(TranscriptSource::Path(PathBuf::from(value)))
    }
}

fn build_latest_url(base: &str, ceremony_id: &str) -> Result<ParsedUrl> {
    let base_url = normalize_base_url(base)?;
    let latest_path = format!("ceremonies/{}/latest.json", ceremony_id);
    base_url
        .join(&latest_path)
        .context("failed to join latest url")
}

fn build_object_url(base: &str, key: &str) -> Result<ParsedUrl> {
    let base_url = normalize_base_url(base)?;
    base_url.join(key).context("failed to join object url")
}

fn normalize_base_url(base: &str) -> Result<ParsedUrl> {
    let mut url = ParsedUrl::parse(base).context("invalid base url")?;
    if !url.path().ends_with('/') {
        let mut path = url.path().to_string();
        path.push('/');
        url.set_path(&path);
    }
    Ok(url)
}

async fn load_transcript(
    client: &reqwest::Client,
    source: TranscriptSource,
    shutdown: &AtomicBool,
) -> Result<Transcript<Bn254>> {
    let bytes = match source {
        TranscriptSource::Url(url) => {
            download_bytes_with_progress(client, url.as_str(), "transcript", shutdown).await?
        }
        TranscriptSource::Path(path) => tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?,
    };

    Transcript::<Bn254>::deserialize_uncompressed(&bytes[..])
        .context("failed to deserialize transcript")
}

/// Load initial transcript from file asynchronously with optimized I/O.
async fn load_initial_transcript_async(path: &Path) -> Result<Transcript<Bn254>> {
    use std::time::Instant;

    let path_display = path.display().to_string();

    // Use async file I/O
    let start = Instant::now();
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read {}", path_display))?;
    println!(
        "Initial transcript file read in {:?} ({} bytes)",
        start.elapsed(),
        bytes.len()
    );

    // Deserialize in a blocking thread to avoid blocking the async runtime
    let start = Instant::now();
    let transcript = tokio::task::spawn_blocking(move || {
        Transcript::<Bn254>::deserialize_uncompressed(&bytes[..])
    })
    .await
    .context("spawn_blocking failed")?
    .context("failed to deserialize initial transcript")?;
    println!(
        "Initial transcript deserialized in {:?} ({} contributions)",
        start.elapsed(),
        transcript.contributions.len()
    );

    Ok(transcript)
}

// ============================================================================
// State Management
// ============================================================================

fn save_state(path: &Path, state: &ContributionState) -> Result<()> {
    let json = serde_json::to_string_pretty(state)?;
    fs::write(path, json)?;
    Ok(())
}

fn load_state(path: &Path) -> Result<ContributionState> {
    let json = fs::read_to_string(path)?;
    let state = serde_json::from_str(&json)?;
    Ok(state)
}

// ============================================================================
// Utility Functions
// ============================================================================

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut result = String::new();
                    result.extend(first.to_uppercase());
                    result.push_str(chars.as_str());
                    result
                }
                None => String::new(),
            }
        })
        .collect()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn build_progress(message: &str, total_bytes: Option<u64>) -> ProgressBar {
    let progress = match total_bytes {
        Some(total) => ProgressBar::new(total),
        None => ProgressBar::new_spinner(),
    };
    progress.set_draw_target(ProgressDrawTarget::stderr());
    progress.enable_steady_tick(Duration::from_millis(120));
    if total_bytes.is_some() {
        let style = ProgressStyle::with_template(
            "{spinner:.green} {msg} [{bar:40}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("=>-")
        .tick_chars("|/-\\");
        progress.set_style(style);
    } else {
        let style = ProgressStyle::with_template("{spinner:.green} {msg} {bytes} downloaded")
            .unwrap()
            .tick_chars("|/-\\");
        progress.set_style(style);
    }
    progress.set_message(message.to_string());
    progress
}
