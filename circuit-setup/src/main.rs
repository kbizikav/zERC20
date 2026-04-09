// SPDX-License-Identifier: BUSL-1.1

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod manifest;
mod s3;

#[derive(Parser)]
#[command(name = "circuit-setup")]
#[command(about = "Circuit artifacts management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate circuit artifacts
    Generate {
        /// Artifact version
        #[arg(long, env = "ARTIFACTS_VERSION")]
        version: String,

        /// Fixed seed for deterministic generation (NOT recommended for production)
        #[arg(long)]
        seed: Option<u64>,

        /// Output directory for artifacts
        #[arg(long, env = "NOVA_ARTIFACTS_DIR")]
        artifacts_dir: Option<PathBuf>,
    },

    /// Upload artifacts to S3
    Upload {
        /// Artifact version
        #[arg(long, env = "ARTIFACTS_VERSION")]
        version: String,

        /// Local artifacts directory
        #[arg(long, env = "NOVA_ARTIFACTS_DIR")]
        artifacts_dir: Option<PathBuf>,

        /// S3 bucket name
        #[arg(long, env = "S3_BUCKET")]
        bucket: String,

        /// S3 prefix (optional)
        #[arg(long, env = "S3_PREFIX", default_value = "")]
        prefix: String,
    },

    /// Download artifacts from public URL
    Download {
        /// Artifact version to download
        #[arg(long, env = "ARTIFACTS_VERSION")]
        version: String,

        /// Local artifacts directory
        #[arg(long, env = "NOVA_ARTIFACTS_DIR")]
        artifacts_dir: Option<PathBuf>,

        /// Base URL for artifacts (e.g., <https://bucket.s3.amazonaws.com/prefix>)
        #[arg(long, env = "ARTIFACTS_BASE_URL")]
        base_url: String,
    },

    /// Generate Solidity verifiers from artifacts
    GenerateVerifier {
        /// Local artifacts directory (input)
        #[arg(long, env = "NOVA_ARTIFACTS_DIR")]
        artifacts_dir: Option<PathBuf>,

        /// Output directory for Solidity files (defaults to artifacts_dir)
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// Test circuit artifacts by generating and verifying dummy proofs
    Test {
        /// Local artifacts directory
        #[arg(long, env = "NOVA_ARTIFACTS_DIR")]
        artifacts_dir: Option<PathBuf>,
    },
}

fn default_artifacts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("nova_artifacts"))
        .unwrap_or_else(|| PathBuf::from("nova_artifacts"))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Generate {
            version,
            seed,
            artifacts_dir,
        } => {
            let artifacts_dir = artifacts_dir.unwrap_or_else(default_artifacts_dir);
            log::info!(
                "Generating artifacts version {} in {}",
                version,
                artifacts_dir.display()
            );
            commands::generate::generate(&artifacts_dir, &version, seed)?;
        }

        Commands::Upload {
            version,
            artifacts_dir,
            bucket,
            prefix,
        } => {
            let artifacts_dir = artifacts_dir.unwrap_or_else(default_artifacts_dir);
            log::info!(
                "Uploading artifacts version {} from {} to s3://{}/{}",
                version,
                artifacts_dir.display(),
                bucket,
                prefix
            );

            let client = s3::create_s3_client().await?;
            let storage = s3::Storage::new(client, bucket, prefix);

            commands::upload::upload(&artifacts_dir, &storage, &version).await?;
        }

        Commands::Download {
            version,
            artifacts_dir,
            base_url,
        } => {
            let artifacts_dir = artifacts_dir.unwrap_or_else(default_artifacts_dir);
            log::info!(
                "Downloading artifacts version {} from {} to {}",
                version,
                base_url,
                artifacts_dir.display()
            );

            commands::download::download(&artifacts_dir, &version, &base_url).await?;
        }

        Commands::GenerateVerifier {
            artifacts_dir,
            output,
        } => {
            let artifacts_dir = artifacts_dir.unwrap_or_else(default_artifacts_dir);
            let output_dir = output.as_deref();
            log::info!(
                "Generating Solidity verifiers from {} to {}",
                artifacts_dir.display(),
                output_dir
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| artifacts_dir.display().to_string())
            );
            commands::verifier::generate_verifiers(&artifacts_dir, output_dir)?;
        }

        Commands::Test { artifacts_dir } => {
            let artifacts_dir = artifacts_dir.unwrap_or_else(default_artifacts_dir);
            log::info!("Testing artifacts in {}", artifacts_dir.display());
            commands::test::test_artifacts(&artifacts_dir)?;
        }
    }

    Ok(())
}
