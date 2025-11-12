use std::{collections::HashMap, env, path::Path, str::FromStr, time::Duration};

use alloy::{
    network::Ethereum,
    primitives::{B256, Bytes, U256},
    providers::PendingTransactionBuilder,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use client_common::{
    contracts::{
        hub::{HubContract, HubTokenInfo},
        utils::{get_provider, get_provider_with_fallback},
        verifier::VerifierContract,
    },
    tokens::{HubEntry, TokenEntry, load_tokens_from_compressed, load_tokens_from_path},
};
use log::{error, info, warn};
use tokio::time::{self, MissedTickBehavior};

#[derive(Parser, Debug)]
#[command(
    name = "crosschain-job",
    about = "Periodically relays transfer roots and broadcasts hub updates across chains"
)]
struct Cli {
    /// Tokens configuration file path.
    #[arg(
        long,
        env = "TOKENS_FILE_PATH",
        value_name = "PATH",
        default_value = "../config/tokens.json"
    )]
    tokens_file_path: std::path::PathBuf,

    /// Private key used to submit relay and broadcast transactions.
    #[arg(long, env = "RELAY_PRIVATE_KEY", value_name = "HEX", required = true)]
    relay_private_key: String,

    /// Interval in seconds between relayTransferRoot submissions per verifier.
    #[arg(
        long,
        env = "RELAY_INTERVAL_SECS",
        value_name = "SECONDS",
        default_value_t = 300
    )]
    relay_interval_secs: u64,

    /// Hex-encoded LayerZero options payload for relayTransferRoot.
    #[arg(long, env = "RELAY_OPTIONS", value_name = "HEX", default_value = "0x")]
    relay_options: HexData,

    /// Additional fee buffer (basis points) applied on top of quoted relay native fee.
    #[arg(
        long,
        env = "RELAY_NATIVE_FEE_BUFFER_BPS",
        value_name = "BPS",
        default_value_t = 1000
    )]
    relay_fee_buffer_bps: u64,

    /// Interval in seconds between Hub.broadcast submissions.
    #[arg(
        long,
        env = "BROADCAST_INTERVAL_SECS",
        value_name = "SECONDS",
        default_value_t = 600
    )]
    broadcast_interval_secs: u64,

    /// Hex-encoded LayerZero options payload for Hub.broadcast.
    #[arg(
        long,
        env = "BROADCAST_OPTIONS",
        value_name = "HEX",
        default_value = "0x"
    )]
    broadcast_options: HexData,

    /// Additional fee buffer (basis points) applied on top of quoted broadcast native fee.
    #[arg(
        long,
        env = "BROADCAST_NATIVE_FEE_BUFFER_BPS",
        value_name = "BPS",
        default_value_t = 1000
    )]
    broadcast_fee_buffer_bps: u64,

    /// Run each job once and exit.
    #[arg(long, env = "JOB_ONCE", default_value_t = false)]
    once: bool,
}

#[derive(Clone, Debug)]
struct HexData(Vec<u8>);

impl FromStr for HexData {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hex_blob(s).map(HexData)
    }
}

impl HexData {
    fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

struct RelayJob {
    label: String,
    chain_id: u64,
    contract: VerifierContract,
    private_key: B256,
    lz_options: Vec<u8>,
    interval: Duration,
    fee_buffer_bps: u64,
}

impl RelayJob {
    async fn run(self) {
        if let Err(err) = self.execute_once().await {
            error!(
                "initial relayTransferRoot for '{}' (chain {}) failed: {err:?}",
                self.label, self.chain_id
            );
        }
        let mut ticker = time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(err) = self.execute_once().await {
                error!(
                    "relayTransferRoot for '{}' (chain {}) failed: {err:?}",
                    self.label, self.chain_id
                );
            }
        }
    }

    async fn execute_once(&self) -> Result<()> {
        let up_to_date = self
            .contract
            .is_up_to_date()
            .await
            .context("failed to query verifier freshness")?;
        if up_to_date {
            info!(
                "skipping relayTransferRoot for '{}' (chain {}) because the verifier is up to date",
                self.label, self.chain_id
            );
            return Ok(());
        }

        let (native_fee, lz_token_fee) = self
            .contract
            .quote_relay(&self.lz_options)
            .await
            .context("failed to quote relay fee")?;

        if lz_token_fee > U256::ZERO {
            warn!(
                "relayTransferRoot quote for '{}' returned non-zero LZ token fee: {lz_token_fee}",
                self.label
            );
        }

        let fee_with_buffer = apply_fee_buffer(native_fee, self.fee_buffer_bps);
        let pending = self
            .contract
            .relay_transfer_root(self.private_key, fee_with_buffer, &self.lz_options)
            .await
            .context("failed to submit relayTransferRoot transaction")?;
        let tx_hash = pending.tx_hash();

        info!(
            "submitted relayTransferRoot for '{}' (chain {}, tx={tx_hash:#x}, fee={} wei, buffer_bps={})",
            self.label, self.chain_id, fee_with_buffer, self.fee_buffer_bps
        );

        let receipt = wait_for_receipt(pending)
            .await
            .context("relayTransferRoot transaction reverted or missing receipt")?;
        match self.contract.parse_transfer_root_relayed(&receipt) {
            Ok((index, root, guid)) => {
                info!(
                    "relayTransferRoot confirmed for '{}' (index={}, root={root:#x}, guid={guid:#x})",
                    self.label, index
                );
            }
            Err(err) => {
                warn!(
                    "relayTransferRoot receipt for '{}' missing event: {err:?}",
                    self.label
                );
            }
        }

        Ok(())
    }
}

struct BroadcastJob {
    contract: HubContract,
    private_key: B256,
    lz_options: Vec<u8>,
    interval: Duration,
    target_eids: Vec<u32>,
    fee_buffer_bps: u64,
}

impl BroadcastJob {
    async fn run(self) {
        if let Err(err) = self.execute_once().await {
            error!("initial Hub.broadcast failed: {err:?}");
        }
        let mut ticker = time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(err) = self.execute_once().await {
                error!("Hub.broadcast failed: {err:?}");
            }
        }
    }

    async fn execute_once(&self) -> Result<()> {
        if self.target_eids.is_empty() {
            warn!("skipping Hub.broadcast because no target EIDs were discovered");
            return Ok(());
        }

        let up_to_date = self
            .contract
            .is_up_to_date()
            .await
            .context("failed to query hub freshness")?;
        if up_to_date {
            info!("skipping Hub.broadcast because the aggregation snapshot is already up to date");
            return Ok(());
        }

        let options = Bytes::copy_from_slice(&self.lz_options);
        let quote = self
            .contract
            .quote_broadcast(self.target_eids.clone(), options.clone())
            .await
            .context("failed to quote broadcast fee")?;

        let fee_with_buffer = apply_fee_buffer(quote, self.fee_buffer_bps);
        let pending = self
            .contract
            .broadcast(
                self.private_key,
                self.target_eids.clone(),
                options,
                fee_with_buffer,
            )
            .await
            .context("failed to submit Hub.broadcast transaction")?;
        let tx_hash = pending.tx_hash();

        info!(
            "submitted Hub.broadcast (tx={tx_hash:#x}, targets={:?}, fee={} wei, buffer_bps={})",
            self.target_eids, fee_with_buffer, self.fee_buffer_bps
        );

        let receipt = wait_for_receipt(pending)
            .await
            .context("Hub.broadcast transaction reverted or missing receipt")?;

        if let Some(event) = parse_broadcast_receipt(&self.contract, &receipt) {
            info!(
                "Hub.broadcast confirmed (agg_seq={}, snapshot_len={})",
                event.agg_seq, event.snapshot_len
            );
        } else {
            warn!("Hub.broadcast receipt did not contain AggregationRootUpdated event");
        }

        Ok(())
    }
}

struct BroadcastReceiptInfo {
    agg_seq: u64,
    snapshot_len: usize,
}

fn parse_broadcast_receipt(
    contract: &HubContract,
    receipt: &alloy::rpc::types::TransactionReceipt,
) -> Option<BroadcastReceiptInfo> {
    match contract.parse_aggregation_root_updated(receipt) {
        Ok(event) => Some(BroadcastReceiptInfo {
            agg_seq: event.agg_seq,
            snapshot_len: event.snapshot.len(),
        }),
        Err(_) => None,
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    if cli.relay_interval_secs == 0 {
        bail!("RELAY_INTERVAL_SECS must be greater than zero");
    }
    if cli.broadcast_interval_secs == 0 && !cli.once {
        bail!("BROADCAST_INTERVAL_SECS must be greater than zero");
    }

    let (tokens, hub_entry) = load_tokens_config(&cli.tokens_file_path)?;
    if tokens.is_empty() {
        bail!(
            "no tokens configured; set TOKENS_COMPRESSED or populate {}",
            cli.tokens_file_path.display()
        );
    }

    let relay_options = cli.relay_options.into_vec();
    let broadcast_options = cli.broadcast_options.into_vec();
    let private_key = parse_private_key(&cli.relay_private_key)?;

    let mut relay_jobs = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let provider = build_provider(&token.rpc_urls)
            .with_context(|| format!("failed to construct provider for token '{}'", token.label))?;
        let contract =
            VerifierContract::new(provider, token.verifier_address).with_legacy_tx(token.legacy_tx);
        relay_jobs.push(RelayJob {
            label: token.label.clone(),
            chain_id: token.chain_id,
            contract,
            private_key,
            lz_options: relay_options.clone(),
            interval: Duration::from_secs(cli.relay_interval_secs),
            fee_buffer_bps: cli.relay_fee_buffer_bps,
        });
    }

    let mut broadcast_job = match hub_entry {
        Some(hub) => {
            let provider = build_provider(&hub.rpc_urls)
                .with_context(|| "failed to construct provider for hub".to_string())?;
            let contract = HubContract::new(provider, hub.hub_address);

            let mut target_eids = resolve_target_eids(&contract, &tokens).await?;
            target_eids.sort_unstable();
            target_eids.dedup();

            Some(BroadcastJob {
                contract,
                private_key,
                lz_options: broadcast_options,
                interval: Duration::from_secs(cli.broadcast_interval_secs.max(1)),
                target_eids,
                fee_buffer_bps: cli.broadcast_fee_buffer_bps,
            })
        }
        None => {
            warn!(
                "hub configuration missing in {} - Hub.broadcast job will be disabled",
                cli.tokens_file_path.display()
            );
            None
        }
    };

    if cli.once {
        for job in relay_jobs {
            job.execute_once()
                .await
                .with_context(|| "relayTransferRoot execution failed in --once mode".to_string())?;
        }

        if let Some(job) = broadcast_job.take() {
            job.execute_once()
                .await
                .with_context(|| "Hub.broadcast execution failed in --once mode".to_string())?;
        }
        return Ok(());
    }

    let mut handles = Vec::new();
    for job in relay_jobs {
        handles.push(tokio::spawn(async move {
            job.run().await;
        }));
    }

    if let Some(job) = broadcast_job {
        handles.push(tokio::spawn(async move {
            job.run().await;
        }));
    }

    info!("crosschain jobs started; waiting for Ctrl+C");
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for Ctrl+C")?;
    info!("Ctrl+C received, shutting down jobs");

    for handle in handles {
        handle.abort();
    }

    Ok(())
}

async fn resolve_target_eids(hub: &HubContract, tokens: &[TokenEntry]) -> Result<Vec<u32>> {
    let hub_infos = hub
        .token_infos()
        .await
        .context("failed to query hub token infos")?;

    let mut infos_by_chain = HashMap::with_capacity(hub_infos.len());
    for info in hub_infos {
        let chain_id = info.chain_id;
        if infos_by_chain.insert(chain_id, info).is_some() {
            warn!(
                "hub reports duplicate token info entries for chain_id {}",
                chain_id
            );
        }
    }

    let mut eids = Vec::with_capacity(tokens.len());
    for token in tokens {
        match infos_by_chain.get(&token.chain_id) {
            Some(info) => {
                log_token_info_mismatch(token, info);
                eids.push(info.eid);
            }
            None => {
                warn!(
                    "token '{}' (chain {}) missing from hub token infos; skipping broadcast target",
                    token.label, token.chain_id
                );
            }
        }
    }

    Ok(eids)
}

fn log_token_info_mismatch(token: &TokenEntry, info: &HubTokenInfo) {
    if info.verifier != token.verifier_address {
        warn!(
            "hub token info verifier mismatch: config '{}' expects {:?}, hub reports {:?}",
            token.label, token.verifier_address, info.verifier
        );
    }
    if info.chain_id != token.chain_id {
        warn!(
            "hub token info chain_id mismatch: config '{}' expects {}, hub reports {}",
            token.label, token.chain_id, info.chain_id
        );
    }
}

fn build_provider(rpc_urls: &[String]) -> Result<client_common::contracts::utils::NormalProvider> {
    if rpc_urls.is_empty() {
        bail!("provider requires at least one RPC URL");
    }
    if rpc_urls.len() == 1 {
        get_provider(
            rpc_urls
                .first()
                .expect("one url validated via length check"),
        )
    } else {
        get_provider_with_fallback(rpc_urls)
    }
}

fn load_tokens_config(path: &Path) -> Result<(Vec<TokenEntry>, Option<HubEntry>)> {
    if let Some(tokens) = load_tokens_config_from_env()? {
        return Ok(tokens);
    }
    let tokens_file = load_tokens_from_path(path)?;
    Ok((tokens_file.tokens, tokens_file.hub))
}

fn load_tokens_config_from_env() -> Result<Option<(Vec<TokenEntry>, Option<HubEntry>)>> {
    match env::var("TOKENS_COMPRESSED") {
        Ok(value) => {
            if value.trim().is_empty() {
                bail!("TOKENS_COMPRESSED is set but empty");
            }
            let tokens_file = load_tokens_from_compressed(&value)
                .context("failed to parse TOKENS_COMPRESSED payload")?;
            Ok(Some((tokens_file.tokens, tokens_file.hub)))
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("TOKENS_COMPRESSED contains invalid unicode")
        }
    }
}

fn parse_private_key(input: &str) -> Result<B256> {
    let normalized = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    let bytes = hex::decode(normalized)
        .with_context(|| format!("failed to decode private key hex: {input}"))?;
    if bytes.len() != 32 {
        bail!("private key must be 32 bytes, got {}", bytes.len());
    }
    Ok(B256::from_slice(&bytes))
}

fn parse_hex_blob(input: &str) -> Result<Vec<u8>> {
    let normalized = input.trim();
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("0x") {
        return Ok(Vec::new());
    }
    let without_prefix = normalized.strip_prefix("0x").unwrap_or(normalized);
    if without_prefix.len() % 2 != 0 {
        bail!("hex string must have even length: {input}");
    }
    hex::decode(without_prefix).with_context(|| format!("failed to decode hex payload: {input}"))
}

async fn wait_for_receipt(
    pending: PendingTransactionBuilder<Ethereum>,
) -> Result<alloy::rpc::types::TransactionReceipt> {
    let receipt = pending
        .get_receipt()
        .await
        .context("failed to fetch transaction receipt")?;
    if receipt.status() {
        Ok(receipt)
    } else {
        bail!("transaction reverted: {:?}", receipt);
    }
}

fn apply_fee_buffer(fee: U256, buffer_bps: u64) -> U256 {
    if buffer_bps == 0 {
        return fee;
    }
    let base = U256::from(10_000u64);
    let multiplier = base + U256::from(buffer_bps);
    let mut buffered = fee.saturating_mul(multiplier) / base;
    if buffered == fee && fee > U256::ZERO {
        buffered = fee + U256::from(1u64);
    }
    buffered
}
