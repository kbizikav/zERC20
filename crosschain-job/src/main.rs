use std::{
    collections::{HashMap, HashSet},
    path::Path,
    str::FromStr,
    time::Duration,
};

use futures::{StreamExt, stream::FuturesUnordered};
use rand::Rng;

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
    layerzero::{HttpLayerZeroClient, LayerZeroClient},
    tokens::{HubEntry, TokenEntry, load_tokens_from_path},
};
use log::{debug, error, info, warn};
use reqwest::Url;
use tokio::time::{self, Instant, MissedTickBehavior, sleep};

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
    /// Can be overridden per token via relay_interval_secs in the tokens config.
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
    /// Can be overridden via broadcast_interval_secs in the hub config.
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

    /// Maximum time in seconds to wait for cross-chain delivery confirmation.
    #[arg(
        long,
        env = "CONFIRMATION_TIMEOUT_SECS",
        value_name = "SECONDS",
        default_value_t = 2400
    )]
    confirmation_timeout_secs: u64,

    /// Poll interval in seconds while waiting for cross-chain delivery confirmation.
    #[arg(
        long,
        env = "CONFIRMATION_POLL_INTERVAL_SECS",
        value_name = "SECONDS",
        default_value_t = 60
    )]
    confirmation_poll_interval_secs: u64,

    /// Optional LayerZero Scan base URL used to inspect stuck messages.
    #[arg(long, env = "LZ_SCAN_API_URL", value_name = "URL")]
    lz_scan_api_url: Option<String>,

    /// Optional LayerZero Scan API key.
    #[arg(long, env = "LZ_SCAN_API_KEY", value_name = "KEY")]
    lz_scan_api_key: Option<String>,

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

#[derive(Clone, Debug)]
struct ConfirmationSettings {
    poll_interval: Duration,
    timeout: Duration,
}

#[derive(Clone)]
struct LayerZeroProbe {
    client: HttpLayerZeroClient,
}

impl LayerZeroProbe {
    async fn log_tx_messages(&self, tx_hash: B256, context: &str) {
        let hash_hex = format!("{tx_hash:#x}");
        match self.client.tx_messages(&hash_hex).await {
            Ok(Some(messages)) => {
                for (idx, message) in messages.data.iter().enumerate() {
                    let status = message
                        .status
                        .as_ref()
                        .and_then(|s| s.name.as_deref())
                        .unwrap_or("<unknown>");
                    let dest_status = message
                        .destination
                        .as_ref()
                        .and_then(|d| d.status.as_deref())
                        .unwrap_or("<unknown>");
                    debug!(
                        "LayerZero Scan status for {context} message #{idx} (tx={hash_hex}): source={status}, destination={dest_status}"
                    );
                }
            }
            Ok(None) => debug!("LayerZero Scan has no message record for tx {hash_hex}"),
            Err(err) => warn!("LayerZero Scan lookup failed for tx {hash_hex}: {err}",),
        }
    }
}

#[derive(Clone, Debug)]
struct HubTokenLayout {
    eid: u32,
    position: u64,
}

#[derive(Clone)]
struct HubContext {
    contract: HubContract,
    layout: HashMap<u64, HubTokenLayout>,
}

#[derive(Clone)]
struct HubRelayDestination {
    hub: HubContract,
    layout: HubTokenLayout,
}

impl HubRelayDestination {
    async fn current_tree_index(&self) -> Result<Option<u64>> {
        // `position` is derived from the Hub's token info array, so treat failures as transient RPC issues.
        let index = self
            .hub
            .transfer_tree_index(self.layout.position)
            .await
            .context("failed to read hub transfer tree index")?;
        Ok(Some(index))
    }

    async fn wait_for_index(
        &self,
        expected_index: u64,
        confirmation: &ConfirmationSettings,
    ) -> Result<bool> {
        let deadline = Instant::now() + confirmation.timeout;
        loop {
            if let Some(current) = self.current_tree_index().await?
                && current >= expected_index
            {
                return Ok(true);
            }

            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(confirmation.poll_interval).await;
        }
    }
}

struct BroadcastDestination {
    label: String,
    chain_id: u64,
    eid: u32,
    verifier: VerifierContract,
}

impl BroadcastDestination {
    /// Check if the verifier's latest global root matches the expected root.
    /// This uses root-based comparison instead of agg_seq to support selective broadcast:
    /// even if agg_seq differs, verifiers with the same root are considered in sync.
    async fn has_current_root(&self, expected_root: U256) -> Result<bool> {
        let latest_seq = self
            .verifier
            .latest_agg_seq()
            .await
            .context("failed to fetch verifier latestAggSeq")?;
        if latest_seq == 0 {
            return Ok(false);
        }
        let latest_root = self
            .verifier
            .global_transfer_root(latest_seq)
            .await
            .context("failed to fetch verifier globalTransferRoots entry")?;
        Ok(latest_root == expected_root && latest_root != U256::ZERO)
    }

    /// Wait for the verifier to receive the expected root (regardless of agg_seq).
    async fn wait_for_root(
        &self,
        expected_root: U256,
        confirmation: &ConfirmationSettings,
    ) -> Result<bool> {
        let deadline = Instant::now() + confirmation.timeout;
        loop {
            if self.has_current_root(expected_root).await? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(confirmation.poll_interval).await;
        }
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
    destination: Option<HubRelayDestination>,
    confirmation: ConfirmationSettings,
    lz_probe: Option<LayerZeroProbe>,
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
        let latest_proved_index = self
            .contract
            .latest_proved_index()
            .await
            .context("failed to fetch latest proved index")?;
        let proved_root = self
            .contract
            .proved_transfer_root(latest_proved_index)
            .await
            .context("failed to fetch proved transfer root")?;
        if proved_root == U256::ZERO {
            info!(
                "skipping relayTransferRoot for '{}' (chain {}) because no proved root exists at index {}",
                self.label, self.chain_id, latest_proved_index
            );
            return Ok(());
        }

        if let Some(dest) = &self.destination {
            if let Some(current_index) = dest.current_tree_index().await?
                && current_index >= latest_proved_index
            {
                info!(
                    "skipping relayTransferRoot for '{}' (chain {}) because hub already has index {}",
                    self.label, self.chain_id, current_index
                );
                return Ok(());
            }
        } else {
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
        let tx_hash = *pending.tx_hash();

        info!(
            "submitted relayTransferRoot for '{}' (chain {}, tx={tx_hash:#x}, fee={} wei, buffer_bps={})",
            self.label, self.chain_id, fee_with_buffer, self.fee_buffer_bps
        );

        let receipt = wait_for_receipt(pending)
            .await
            .context("relayTransferRoot transaction reverted or missing receipt")?;
        let mut relay_event = None;
        match self.contract.parse_transfer_root_relayed(&receipt) {
            Ok((index, root, guid)) => {
                info!(
                    "relayTransferRoot confirmed for '{}' (index={}, root={root:#x}, guid={guid:#x})",
                    self.label, index
                );
                relay_event = Some((index, root));
            }
            Err(err) => {
                warn!(
                    "relayTransferRoot receipt for '{}' missing event: {err:?}",
                    self.label
                );
            }
        }

        if let (Some(dest), Some((index, root))) = (&self.destination, relay_event) {
            let delivered = dest.wait_for_index(index, &self.confirmation).await?;
            if delivered {
                info!(
                    "hub applied transfer root for '{}' (index={}, root={root:#x})",
                    self.label, index
                );
            } else {
                warn!(
                    "hub did not reflect transfer root for '{}' (index={}) before timeout; will retry on the next loop",
                    self.label, index
                );
                if let Some(probe) = &self.lz_probe {
                    probe.log_tx_messages(tx_hash, "relayTransferRoot").await;
                }
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
    last_broadcast_root: Option<U256>,
    last_broadcast_seq: Option<u64>,
    destinations: Vec<BroadcastDestination>,
    confirmation: ConfirmationSettings,
    lz_probe: Option<LayerZeroProbe>,
}

impl BroadcastJob {
    async fn run(mut self) {
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

    async fn execute_once(&mut self) -> Result<()> {
        if self.target_eids.is_empty() {
            warn!("skipping Hub.broadcast because no target EIDs were discovered");
            return Ok(());
        }

        let agg_seq = self
            .contract
            .agg_seq()
            .await
            .context("failed to fetch hub aggSeq")?;
        let current_root = self
            .contract
            .current_aggregation_root()
            .await
            .context("failed to compute current aggregation root")?;
        let missing_targets = self
            .destinations_missing(current_root)
            .await
            .context("failed to evaluate broadcast destinations")?;

        if missing_targets.is_empty() && self.last_broadcast_root.is_none() {
            // Align local cache when everything is already in sync to avoid a redundant rebroadcast on startup.
            self.last_broadcast_root = Some(current_root);
            self.last_broadcast_seq = Some(agg_seq);
            info!(
                "skipping Hub.broadcast because all verifiers already have agg_seq {} root {current_root:#x}",
                agg_seq
            );
            return Ok(());
        }

        let root_changed = self
            .last_broadcast_root
            .map(|root| root != current_root)
            .unwrap_or(true);

        if !root_changed && missing_targets.is_empty() {
            info!(
                "skipping Hub.broadcast because aggregation root {current_root:#x} is already delivered to all targets"
            );
            return Ok(());
        }

        // Selective broadcast: only send to verifiers that don't have the current root.
        // This uses root-based comparison (not agg_seq) to avoid convergence issues:
        // verifiers with the same root are considered in sync even if their agg_seq differs.
        let (broadcast_eids, is_selective) = if root_changed {
            // Root changed: broadcast to all targets
            (self.target_eids.clone(), false)
        } else {
            // Root unchanged but some targets missing: broadcast only to missing targets
            let missing_eids: Vec<u32> = missing_targets.iter().map(|(_, eid)| *eid).collect();
            (missing_eids, true)
        };

        let options = Bytes::copy_from_slice(&self.lz_options);
        let quote = self
            .contract
            .quote_broadcast(broadcast_eids.clone(), options.clone())
            .await
            .context("failed to quote broadcast fee")?;

        let fee_with_buffer = apply_fee_buffer(quote, self.fee_buffer_bps);
        let pending = self
            .contract
            .broadcast(
                self.private_key,
                broadcast_eids.clone(),
                options,
                fee_with_buffer,
            )
            .await
            .context("failed to submit Hub.broadcast transaction")?;
        let tx_hash = *pending.tx_hash();

        if is_selective {
            let missing_labels: Vec<&str> = missing_targets
                .iter()
                .map(|(label, _)| label.as_str())
                .collect();
            info!(
                "submitted selective Hub.broadcast to missing targets {:?} (tx={tx_hash:#x}, eids={:?}, fee={} wei, buffer_bps={})",
                missing_labels, broadcast_eids, fee_with_buffer, self.fee_buffer_bps
            );
        } else {
            info!(
                "submitted Hub.broadcast (tx={tx_hash:#x}, targets={:?}, fee={} wei, buffer_bps={})",
                broadcast_eids, fee_with_buffer, self.fee_buffer_bps
            );
        }

        let receipt = wait_for_receipt(pending)
            .await
            .context("Hub.broadcast transaction reverted or missing receipt")?;

        if let Some(event) = parse_broadcast_receipt(&self.contract, &receipt) {
            let target_labels: Vec<&str> = self
                .destinations
                .iter()
                .filter(|d| broadcast_eids.contains(&d.eid))
                .map(|d| d.label.as_str())
                .collect();
            info!(
                "Hub.broadcast confirmed (agg_seq={}, snapshot_len={}, root={:#x}, targets={:?})",
                event.agg_seq, event.snapshot_len, event.root, target_labels
            );
            self.last_broadcast_root = Some(event.root);
            self.last_broadcast_seq = Some(event.agg_seq);
            self.confirm_destinations(event.root, &broadcast_eids, tx_hash)
                .await;
        } else {
            warn!("Hub.broadcast receipt did not contain AggregationRootUpdated event");
            self.last_broadcast_root = Some(current_root);
            self.last_broadcast_seq = Some(agg_seq.saturating_add(1));
            if let Some(probe) = &self.lz_probe {
                probe.log_tx_messages(tx_hash, "Hub.broadcast").await;
            }
        }

        Ok(())
    }

    async fn destinations_missing(&self, current_root: U256) -> Result<Vec<(String, u32)>> {
        let jitters = random_jitters(self.destinations.len());
        let futs: FuturesUnordered<_> = self
            .destinations
            .iter()
            .zip(jitters)
            .map(|(dest, jitter)| async move {
                sleep(jitter).await;
                let result = dest.has_current_root(current_root).await;
                (dest, result)
            })
            .collect();

        let results: Vec<_> = futs.collect().await;

        let mut missing = Vec::new();
        for (dest, result) in results {
            match result {
                Ok(true) => continue,
                Ok(false) => missing.push((dest.label.clone(), dest.eid)),
                Err(err) => {
                    warn!(
                        "failed to check global root on verifier '{}' (chain {}, eid {}): {err:?}",
                        dest.label, dest.chain_id, dest.eid
                    );
                    missing.push((dest.label.clone(), dest.eid));
                }
            }
        }
        Ok(missing)
    }

    async fn confirm_destinations(&self, root: U256, broadcast_eids: &[u32], tx_hash: B256) {
        let eid_set: HashSet<u32> = broadcast_eids.iter().copied().collect();
        let targets: Vec<_> = self
            .destinations
            .iter()
            .filter(|d| eid_set.contains(&d.eid))
            .collect();
        let jitters = random_jitters(targets.len());
        let futs: FuturesUnordered<_> = targets
            .into_iter()
            .zip(jitters)
            .map(|(dest, jitter)| async move {
                sleep(jitter).await;
                let result = dest.wait_for_root(root, &self.confirmation).await;
                (dest, result)
            })
            .collect();

        let results: Vec<_> = futs.collect().await;

        let mut any_timed_out = false;
        for (dest, result) in results {
            match result {
                Ok(true) => {
                    info!(
                        "verifier '{}' (eid {}) confirmed root {root:#x}",
                        dest.label, dest.eid
                    );
                }
                Ok(false) => {
                    warn!(
                        "verifier '{}' (eid {}) did not receive root {root:#x} before timeout; will rebroadcast",
                        dest.label, dest.eid
                    );
                    any_timed_out = true;
                }
                Err(err) => {
                    warn!(
                        "failed to confirm root on verifier '{}' (chain {}, eid {}): {err:?}",
                        dest.label, dest.chain_id, dest.eid
                    );
                }
            }
        }
        if any_timed_out {
            if let Some(probe) = &self.lz_probe {
                probe.log_tx_messages(tx_hash, "Hub.broadcast").await;
            }
        }
    }
}

struct BroadcastReceiptInfo {
    agg_seq: u64,
    snapshot_len: usize,
    root: U256,
}

fn parse_broadcast_receipt(
    contract: &HubContract,
    receipt: &alloy::rpc::types::TransactionReceipt,
) -> Option<BroadcastReceiptInfo> {
    match contract.parse_aggregation_root_updated(receipt) {
        Ok(event) => Some(BroadcastReceiptInfo {
            agg_seq: event.agg_seq,
            snapshot_len: event.snapshot.len(),
            root: event.root,
        }),
        Err(_) => None,
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    if cli.confirmation_timeout_secs == 0 {
        bail!("CONFIRMATION_TIMEOUT_SECS must be greater than zero");
    }
    if cli.confirmation_poll_interval_secs == 0 {
        bail!("CONFIRMATION_POLL_INTERVAL_SECS must be greater than zero");
    }

    let (tokens, hub_entry) = load_tokens_config(&cli.tokens_file_path)?;
    if tokens.is_empty() {
        bail!("no tokens configured in {}", cli.tokens_file_path.display());
    }
    for token in &tokens {
        let interval = token.relay_interval_secs.unwrap_or(cli.relay_interval_secs);
        if interval == 0 {
            bail!(
                "relay interval must be greater than zero for token '{}' (chain {})",
                token.label,
                token.chain_id
            );
        }
    }
    if !cli.once
        && let Some(hub) = &hub_entry
    {
        let interval = hub
            .broadcast_interval_secs
            .unwrap_or(cli.broadcast_interval_secs);
        if interval == 0 {
            bail!(
                "broadcast interval must be greater than zero for hub chain {}",
                hub.chain_id
            );
        }
    }

    let relay_options = cli.relay_options.clone().into_vec();
    let broadcast_options = cli.broadcast_options.clone().into_vec();
    let private_key = parse_private_key(&cli.relay_private_key)?;
    let confirmation = ConfirmationSettings {
        poll_interval: Duration::from_secs(cli.confirmation_poll_interval_secs),
        timeout: Duration::from_secs(cli.confirmation_timeout_secs),
    };
    let lz_probe = build_layerzero_probe(&cli)?;

    let mut hub_context: Option<HubContext> = None;
    let mut broadcast_job = match hub_entry {
        Some(hub) => {
            let provider = build_provider(&hub.rpc_urls)
                .with_context(|| "failed to construct provider for hub".to_string())?;
            let contract =
                HubContract::new(provider, hub.hub_address).with_legacy_tx(hub.legacy_tx);

            let (mut target_eids, layout) = resolve_target_eids(&contract, &tokens).await?;
            target_eids.sort_unstable();
            target_eids.dedup();

            let destinations = build_broadcast_destinations(&tokens, &layout)?;

            hub_context = Some(HubContext {
                contract: contract.clone(),
                layout,
            });

            Some(BroadcastJob {
                contract,
                private_key,
                lz_options: broadcast_options,
                interval: Duration::from_secs(
                    hub.broadcast_interval_secs
                        .unwrap_or(cli.broadcast_interval_secs)
                        .max(1),
                ),
                target_eids,
                fee_buffer_bps: cli.broadcast_fee_buffer_bps,
                last_broadcast_root: None,
                last_broadcast_seq: None,
                destinations,
                confirmation: confirmation.clone(),
                lz_probe: lz_probe.clone(),
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

    let mut relay_jobs = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let provider = build_provider(&token.rpc_urls)
            .with_context(|| format!("failed to construct provider for token '{}'", token.label))?;
        let contract =
            VerifierContract::new(provider, token.verifier_address).with_legacy_tx(token.legacy_tx);

        let destination = hub_context.as_ref().and_then(|ctx| {
            ctx.layout
                .get(&token.chain_id)
                .map(|layout| HubRelayDestination {
                    hub: ctx.contract.clone(),
                    layout: layout.clone(),
                })
        });

        relay_jobs.push(RelayJob {
            label: token.label.clone(),
            chain_id: token.chain_id,
            contract,
            private_key,
            lz_options: relay_options.clone(),
            interval: Duration::from_secs(
                token
                    .relay_interval_secs
                    .unwrap_or(cli.relay_interval_secs)
                    .max(1),
            ),
            fee_buffer_bps: cli.relay_fee_buffer_bps,
            destination,
            confirmation: confirmation.clone(),
            lz_probe: lz_probe.clone(),
        });
    }

    if cli.once {
        for job in relay_jobs {
            job.execute_once()
                .await
                .with_context(|| "relayTransferRoot execution failed in --once mode".to_string())?;
        }

        if let Some(job) = broadcast_job.take() {
            let mut job = job;
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

async fn resolve_target_eids(
    hub: &HubContract,
    tokens: &[TokenEntry],
) -> Result<(Vec<u32>, HashMap<u64, HubTokenLayout>)> {
    let hub_infos = hub
        .token_infos()
        .await
        .context("failed to query hub token infos")?;

    let mut infos_by_chain = HashMap::with_capacity(hub_infos.len());
    for (idx, info) in hub_infos.into_iter().enumerate() {
        let chain_id = info.chain_id;
        if infos_by_chain
            .insert(chain_id, (info, idx as u64))
            .is_some()
        {
            warn!(
                "hub reports duplicate token info entries for chain_id {}",
                chain_id
            );
        }
    }

    let mut eids = Vec::with_capacity(tokens.len());
    let mut layout = HashMap::with_capacity(tokens.len());
    for token in tokens {
        match infos_by_chain.get(&token.chain_id) {
            Some((info, position)) => {
                log_token_info_mismatch(token, info);
                eids.push(info.eid);
                layout.insert(
                    token.chain_id,
                    HubTokenLayout {
                        eid: info.eid,
                        position: *position,
                    },
                );
            }
            None => {
                warn!(
                    "token '{}' (chain {}) missing from hub token infos; skipping broadcast target",
                    token.label, token.chain_id
                );
            }
        }
    }

    Ok((eids, layout))
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

fn build_broadcast_destinations(
    tokens: &[TokenEntry],
    layout: &HashMap<u64, HubTokenLayout>,
) -> Result<Vec<BroadcastDestination>> {
    let mut destinations = Vec::with_capacity(tokens.len());
    for token in tokens {
        let Some(info) = layout.get(&token.chain_id) else {
            continue;
        };

        let provider = build_provider(&token.rpc_urls).with_context(|| {
            format!(
                "failed to construct provider for broadcast destination '{}'",
                token.label
            )
        })?;

        let contract =
            VerifierContract::new(provider, token.verifier_address).with_legacy_tx(token.legacy_tx);
        destinations.push(BroadcastDestination {
            label: token.label.clone(),
            chain_id: token.chain_id,
            eid: info.eid,
            verifier: contract,
        });
    }

    Ok(destinations)
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
    let tokens_file = load_tokens_from_path(path)?;
    Ok((tokens_file.tokens, tokens_file.hub))
}

fn build_layerzero_probe(cli: &Cli) -> Result<Option<LayerZeroProbe>> {
    let Some(url) = cli.lz_scan_api_url.as_ref() else {
        return Ok(None);
    };

    let parsed =
        Url::parse(url).with_context(|| format!("invalid LZ_SCAN_API_URL provided: {url}"))?;
    let client = HttpLayerZeroClient::new(parsed, cli.lz_scan_api_key.clone())
        .context("failed to construct LayerZero Scan client")?;
    Ok(Some(LayerZeroProbe { client }))
}

fn parse_private_key(input: &str) -> Result<B256> {
    let normalized = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    let bytes =
        hex::decode(normalized).context("failed to decode private key: invalid hex format")?;
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
    if !without_prefix.len().is_multiple_of(2) {
        bail!("hex string must have even length: {input}");
    }
    hex::decode(without_prefix).with_context(|| format!("failed to decode hex payload: {input}"))
}

/// Maximum time to wait for a transaction receipt before giving up.
const RECEIPT_TIMEOUT_SECS: u64 = 300; // 5 minutes

async fn wait_for_receipt(
    pending: PendingTransactionBuilder<Ethereum>,
) -> Result<alloy::rpc::types::TransactionReceipt> {
    let timeout_duration = Duration::from_secs(RECEIPT_TIMEOUT_SECS);
    let receipt = time::timeout(timeout_duration, pending.get_receipt())
        .await
        .context("timed out waiting for transaction receipt")?
        .context("failed to fetch transaction receipt")?;
    if receipt.status() {
        Ok(receipt)
    } else {
        bail!("transaction reverted: {:?}", receipt);
    }
}

/// Generate random jitter durations (0-500ms) for staggering concurrent RPC
/// calls. Uses `thread_rng` synchronously so the returned `Vec` is `Send`.
fn random_jitters(count: usize) -> Vec<Duration> {
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| Duration::from_millis(rng.gen_range(0..500)))
        .collect()
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
