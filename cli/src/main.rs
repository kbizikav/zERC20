use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

mod commands;
mod proof;

use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use client_common::{
    indexer::HttpIndexerClient,
    prover::HttpDeciderClient,
    tokens::{HubEntry, TokenEntry, TokensFile},
};
use commands::{
    invoice, private_transfer, receive_transfer, scan_receive_transfers,
    shared::{parse_address, parse_b256, parse_u256},
    transfer,
};
use hex;
use reqwest::Url;

#[derive(Parser, Debug)]
#[command(author, version, about = "zERC20 modern CLI")]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Tokens configuration JSON (required if TOKENS_FILE_PATH env unset).
    #[arg(long, env = "TOKENS_FILE_PATH", value_name = "PATH", required = true)]
    pub tokens_file_path: PathBuf,

    /// Private key used to sign transactions (required if PRIVATE_KEY env unset).
    #[arg(long, env = "PRIVATE_KEY", value_name = "HEX", required = true)]
    pub private_key: String,

    /// Indexer endpoint URL.
    #[arg(long, env = "INDEXER_URL", value_name = "URL", required = true)]
    pub indexer_url: String,

    /// Decider prover endpoint URL.
    #[arg(long, env = "DECIDER_PROVER_URL", value_name = "URL", required = true)]
    pub decider_prover_url: String,

    /// Internet Computer replica base URL used for stealth storage.
    #[arg(long, env = "IC_REPLICA_URL", value_name = "URL", required = true)]
    pub ic_replica_url: String,

    /// Key manager canister principal in text format.
    #[arg(
        long,
        env = "KEY_MANAGER_CANISTER_ID",
        value_name = "PRINCIPAL",
        required = true
    )]
    pub key_manager_canister_id: String,

    /// Storage canister principal in text format.
    #[arg(
        long,
        env = "STORAGE_CANISTER_ID",
        value_name = "PRINCIPAL",
        required = true
    )]
    pub storage_canister_id: String,

    /// Maximum number of events to fetch per burn address.
    #[arg(long, env = "INDEXER_FETCH_LIMIT", default_value_t = 20)]
    pub indexer_fetch_limit: usize,

    /// Number of blocks to span when querying for events.
    #[arg(long, env = "EVENT_BLOCK_SPAN", default_value_t = 5000)]
    pub event_block_span: u64,

    /// Directory containing Nova prover artifacts (defaults to workspace nova_artifacts/).
    #[arg(long, env = "NOVA_ARTIFACTS_DIR", value_name = "PATH")]
    pub nova_artifacts_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Invoice management helpers backed by the storage canister.
    #[command(subcommand)]
    Invoice(InvoiceCommand),
    /// Execute a public ERC-20 transfer.
    Transfer(TransferArgs),
    /// Execute a stealthy burn transfer via FullBurnAddress.
    PrivateTransfer(PrivateTransferArgs),
    /// Redeem a previously generated FullBurnAddress.
    ReceiveTransfer(ReceiveTransferArgs),
    /// Scan storage announcements and persist inbound transfers locally.
    ScanReceiveTransfers(ScanReceiveTransfersArgs),
}

#[derive(Subcommand, Debug)]
enum InvoiceCommand {
    /// List invoice IDs recorded in the storage canister for the caller.
    Ls(InvoiceListArgs),
    /// Issue a new invoice, persist it in the storage canister, and print burn addresses.
    Issue(InvoiceIssueArgs),
    /// Redeem funds for a specific invoice.
    Receive(InvoiceReceiveArgs),
    /// Display eligible transfer events for an invoice without submitting proofs.
    Status(InvoiceReceiveArgs),
}

#[derive(Args, Debug, Clone)]
pub struct TransferArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Destination address for the transfer.
    #[arg(long, value_parser = parse_address)]
    pub to: Address,

    /// Token amount (accepts decimal or 0x-prefixed hex units).
    #[arg(long, value_parser = parse_u256)]
    pub amount: U256,
}

#[derive(Args, Debug, Clone)]
pub struct PrivateTransferArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Destination address for the stealth transfer.
    #[arg(long, value_parser = parse_address, value_name = "ADDRESS")]
    pub to: Address,

    /// Target chain id encoded into the FullBurnAddress payload.
    #[arg(long, value_name = "CHAIN_ID")]
    pub to_chain_id: u64,

    /// Token amount (accepts decimal or 0x-prefixed hex units).
    #[arg(long, value_parser = parse_u256)]
    pub amount: U256,
}

#[derive(Args, Debug, Clone)]
pub struct ScanReceiveTransfersArgs {
    /// Path to write decrypted FullBurnAddress payloads.
    #[arg(
        long,
        env = "SCAN_RECEIVE_OUTPUT",
        value_name = "PATH",
        default_value = "output.json"
    )]
    pub output: PathBuf,

    /// Storage announcement page size.
    #[arg(long, env = "SCAN_PAGE_SIZE", default_value_t = 100)]
    pub page_size: u32,

    /// Authorization TTL in seconds for requesting the encrypted view key.
    #[arg(long, env = "SCAN_AUTHORIZATION_TTL", default_value_t = 600)]
    pub authorization_ttl_seconds: u64,
}

#[derive(Args, Debug, Clone)]
pub struct ReceiveTransferArgs {
    /// Serialized FullBurnAddress payload in hex.
    #[arg(
        long,
        value_name = "HEX",
        required_unless_present = "announcement_id",
        conflicts_with = "announcement_id"
    )]
    pub full_burn_address: Option<String>,

    /// Announcement identifier saved by scan_receive_transfers.
    #[arg(
        long,
        value_name = "ID",
        required_unless_present = "full_burn_address",
        conflicts_with = "full_burn_address"
    )]
    pub announcement_id: Option<u64>,

    /// Path to the JSON output produced by scan_receive_transfers (required when using --announcement-id).
    #[arg(
        long,
        env = "SCAN_RECEIVE_OUTPUT",
        value_name = "PATH",
        default_value = "output.json"
    )]
    pub scan_results_path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct InvoiceListArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Optional override address for listing (defaults to signer address).
    #[arg(long, value_parser = parse_address, value_name = "ADDRESS")]
    pub owner: Option<Address>,
}

#[derive(Args, Debug, Clone)]
pub struct InvoiceIssueArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Optional override for the recipient address (defaults to signer address).
    #[arg(long, value_parser = parse_address, value_name = "ADDRESS")]
    pub recipient: Option<Address>,

    /// Enable batch mode (prints ten burn addresses).
    #[arg(long, default_value_t = false)]
    pub batch: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InvoiceReceiveArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Invoice identifier to redeem.
    #[arg(long, value_name = "INVOICE_ID", value_parser = parse_b256)]
    pub invoice_id: B256,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();
    let cli = Cli::parse();

    let LoadedTokens { tokens, hub } = load_tokens_config(&cli.common.tokens_file_path)
        .with_context(|| "failed to load tokens configuration")?;
    let private_key = parse_private_key(&cli.common.private_key)?;

    match &cli.command {
        Command::Invoice(InvoiceCommand::Ls(args)) => {
            invoice::list(&cli.common, args, &tokens, private_key).await?
        }
        Command::Invoice(InvoiceCommand::Issue(args)) => {
            invoice::issue(&cli.common, args, &tokens, private_key).await?
        }
        Command::Invoice(InvoiceCommand::Receive(args)) => {
            invoice::receive(&cli.common, args, &tokens, hub.as_ref(), private_key).await?
        }
        Command::Invoice(InvoiceCommand::Status(args)) => {
            invoice::status(&cli.common, args, &tokens, hub.as_ref(), private_key).await?
        }
        Command::Transfer(args) => transfer::run(args, &tokens, private_key).await?,
        Command::PrivateTransfer(args) => {
            private_transfer::run(&cli.common, args, &tokens, private_key).await?
        }
        Command::ReceiveTransfer(args) => {
            receive_transfer::run(&cli.common, args, &tokens, hub.as_ref(), private_key).await?
        }
        Command::ScanReceiveTransfers(args) => {
            scan_receive_transfers::run(&cli.common, args, &tokens, private_key).await?
        }
    }

    Ok(())
}

fn parse_private_key(input: &str) -> Result<B256> {
    let normalized = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    let bytes = hex::decode(normalized)
        .with_context(|| format!("failed to decode private key hex: {input}"))?;
    if bytes.len() != 32 {
        bail!("private key must be 32 bytes, got {}", bytes.len());
    }
    let array: [u8; 32] = bytes.try_into().expect("length checked above");
    Ok(B256::from(array))
}

struct LoadedTokens {
    tokens: Vec<TokenEntry>,
    hub: Option<HubEntry>,
}

fn load_tokens_config(path: &Path) -> Result<LoadedTokens> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read tokens config {}", path.display()))?;
    let mut tokens_file: TokensFile =
        serde_json::from_str(&contents).context("failed to parse tokens config JSON")?;
    tokens_file.normalize().context("invalid tokens config")?;
    let mut tokens = tokens_file.tokens;
    for token in tokens.iter_mut() {
        token
            .normalize()
            .with_context(|| format!("invalid token entry '{}'", token.label))?;
    }
    Ok(LoadedTokens {
        tokens,
        hub: tokens_file.hub,
    })
}

fn build_indexer_client(common: &CommonArgs, context_label: &str) -> Result<HttpIndexerClient> {
    let base = Url::parse(&common.indexer_url).with_context(|| {
        format!(
            "invalid INDEXER_URL '{}' for {}",
            common.indexer_url, context_label
        )
    })?;
    HttpIndexerClient::new(base).context("failed to construct indexer client")
}

fn build_decider_client(common: &CommonArgs, context_label: &str) -> Result<HttpDeciderClient> {
    let base = Url::parse(&common.decider_prover_url).with_context(|| {
        format!(
            "invalid DECIDER_PROVER_URL '{}' for {}",
            common.decider_prover_url, context_label
        )
    })?;
    HttpDeciderClient::with_defaults(base, Duration::from_secs(300))
        .context("failed to construct decider prover client")
}
