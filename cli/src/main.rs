use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

mod commands;
mod proof;

use alloy::primitives::{Address, B256, Bytes, U256};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use client_common::{
    indexer::HttpIndexerClient,
    prover::HttpDeciderClient,
    tokens::{HubEntry, TokenEntry, parse_tokens_config},
};
use commands::{
    balance, invoice, lz_status, private_transfer, quote_unwrap, receive_transfer,
    scan_receive_transfers,
    shared::{parse_address, parse_b256, parse_bytes, parse_u256},
    transfer, unwrap, wrap,
};
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

    /// LayerZero Scan API base URL (defaults to testnet).
    #[arg(
        long,
        env = "LZ_SCAN_API_URL",
        value_name = "URL",
        default_value = "https://scan-testnet.layerzero-api.com/v1"
    )]
    pub lz_scan_api_url: String,

    /// Optional LayerZero Scan API key for authenticated access.
    #[arg(long, env = "LZ_SCAN_API_KEY", value_name = "API_KEY")]
    pub lz_scan_api_key: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Invoice management helpers backed by the storage canister.
    #[command(subcommand)]
    Invoice(InvoiceCommand),
    /// Display zERC20 and underlying token balances for the configured account.
    Balance(BalanceArgs),
    /// Execute a public ERC-20 transfer.
    Transfer(TransferArgs),
    /// Execute a stealthy burn transfer via FullBurnAddress.
    PrivateTransfer(PrivateTransferArgs),
    /// Redeem a previously generated FullBurnAddress.
    ReceiveTransfer(ReceiveTransferArgs),
    /// Scan storage announcements and persist inbound transfers locally.
    ScanReceiveTransfers(ScanReceiveTransfersArgs),
    /// Wrap underlying tokens into zERC20 via the liquidity manager.
    Wrap(WrapArgs),
    /// Quote unwrap + bridge fees for all adaptor-enabled chains.
    QuoteUnwrap(QuoteUnwrapArgs),
    /// Unwrap zERC20 locally or via adaptor compose.
    Unwrap(UnwrapArgs),
    /// Display LayerZero message status for the signer wallet.
    LzStatus(LzStatusArgs),
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
pub struct BalanceArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,
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

    #[command(flatten)]
    pub relay: RelayArgs,
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

    /// Tag used to filter announcements.
    #[arg(long, default_value = "v1", value_name = "TAG")]
    pub tag: String,
}

#[derive(Args, Debug, Clone)]
pub struct WrapArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Token amount to wrap (accepts decimal or 0x-prefixed hex units).
    #[arg(long, value_parser = parse_u256)]
    pub amount: U256,

    /// Receiver address for the wrapped zERC20 (defaults to signer).
    #[arg(long, value_parser = parse_address)]
    pub receiver: Option<Address>,
}

#[derive(Args, Debug, Clone)]
pub struct QuoteUnwrapArgs {
    /// Amount of zERC20 to unwrap and bridge (accepts decimal or 0x-prefixed hex units).
    #[arg(long, value_parser = parse_u256)]
    pub amount: U256,

    /// Destination chain identifier; resolved to the LayerZero EID from tokens.json.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub dst_chain_id: u64,

    /// Gas limit used on the destination lzReceive; encoded into extraOptions.
    #[arg(long, default_value_t = 200000)]
    pub lz_receive_gas: u32,

    /// msg.value forwarded to lzReceive on the destination chain.
    #[arg(long, default_value_t = 0)]
    pub lz_receive_value: u128,

    /// Gas limit used on the destination lzCompose; encoded into extraOptions when set.
    #[arg(long, value_name = "GAS")]
    pub lz_compose_gas: Option<u32>,

    /// msg.value forwarded to lzCompose on the destination chain.
    #[arg(long, value_name = "WEI")]
    pub lz_compose_value: Option<u128>,

    /// Recipient of the bridged underlying token (defaults to signer).
    #[arg(long, value_parser = parse_address)]
    pub receiver: Option<Address>,

    /// Address receiving any unused native bridge fee refund (defaults to signer).
    #[arg(long, value_parser = parse_address)]
    pub refund_address: Option<Address>,

    /// Minimum amount expected on destination (defaults to the input amount).
    #[arg(long, value_parser = parse_u256)]
    pub min_amount_out: Option<U256>,

    /// Optional compose message payload passed through Stargate.
    #[arg(long, value_parser = parse_bytes)]
    pub compose_msg: Option<Bytes>,

    /// Optional OFT command payload passed through Stargate.
    #[arg(long, value_parser = parse_bytes)]
    pub oft_cmd: Option<Bytes>,
}

#[derive(Args, Debug, Clone)]
pub struct UnwrapArgs {
    /// Chain identifier used to select the source token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Destination chain identifier; unwraps locally when matching the source.
    #[arg(long, value_name = "CHAIN_ID")]
    pub dst_chain_id: u64,

    /// Amount of zERC20 to unwrap (accepts decimal or 0x-prefixed hex units).
    #[arg(long, value_parser = parse_u256)]
    pub amount: U256,

    #[command(flatten)]
    pub relay: RelayArgs,
}

#[derive(Args, Debug, Clone)]
pub struct UnwrapStatusArgs {
    /// Source chain transaction hash emitted by the cross-chain unwrap.
    #[arg(long, value_name = "TX_HASH")]
    pub tx_hash: String,
}

#[derive(Args, Debug, Clone)]
pub struct LzStatusArgs {
    /// Maximum number of messages to fetch for the signer wallet.
    #[arg(long, default_value_t = 20, value_name = "COUNT")]
    pub limit: usize,

    /// Start date in ISO-8601 format to filter messages.
    #[arg(long, value_name = "ISO8601")]
    pub start: Option<String>,

    /// End date in ISO-8601 format to filter messages.
    #[arg(long, value_name = "ISO8601")]
    pub end: Option<String>,

    /// Pagination token returned by previous calls.
    #[arg(long, value_name = "TOKEN")]
    pub next_token: Option<String>,
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

    #[command(flatten)]
    pub relay: RelayArgs,
}

#[derive(Args, Debug, Clone)]
pub struct InvoiceListArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Optional override address for listing (defaults to signer address).
    #[arg(long, value_parser = parse_address, value_name = "ADDRESS")]
    pub owner: Option<Address>,

    /// Tag used to filter invoices.
    #[arg(long, default_value = "v1", value_name = "TAG")]
    pub tag: String,
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

    /// Tag used when issuing invoices.
    #[arg(long, default_value = "v1", value_name = "TAG")]
    pub tag: String,
}

#[derive(Args, Debug, Clone)]
pub struct InvoiceReceiveArgs {
    /// Chain identifier used to select the token entry.
    #[arg(long, env = "CHAIN_ID", value_name = "CHAIN_ID")]
    pub chain_id: u64,

    /// Invoice identifier to redeem.
    #[arg(long, value_name = "INVOICE_ID", value_parser = parse_b256)]
    pub invoice_id: B256,

    /// Use the chain-local proved transfer root instead of the hub global transfer root.
    #[arg(long, default_value_t = false)]
    pub local: bool,

    #[command(flatten)]
    pub relay: RelayArgs,
}

/// Shared relay-mode arguments for Gelato gasless transactions.
#[derive(Args, Debug, Clone)]
pub struct RelayArgs {
    /// Use Gelato Relay for gasless teleport submission.
    #[arg(long, default_value_t = false)]
    pub relay: bool,

    /// Optional Gelato API key for relay requests.
    #[arg(long, env = "GELATO_API_KEY", value_name = "API_KEY")]
    pub gelato_api_key: Option<String>,

    /// Maximum relayer fee in zERC20 units (optional safety cap).
    #[arg(long, value_parser = parse_u256, value_name = "AMOUNT")]
    pub max_relay_fee: Option<U256>,
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
        Command::Balance(args) => balance::run(args, &tokens, private_key).await?,
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
        Command::Wrap(args) => wrap::run(args, &tokens, private_key).await?,
        Command::QuoteUnwrap(args) => quote_unwrap::run(args, &tokens, private_key).await?,
        Command::Unwrap(args) => unwrap::run(args, &tokens, private_key).await?,
        Command::LzStatus(args) => lz_status::run(&cli.common, args, private_key).await?,
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
    let tokens_file = parse_tokens_config(&contents)
        .with_context(|| format!("invalid tokens config {}", path.display()))?;
    Ok(LoadedTokens {
        tokens: tokens_file.tokens,
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
