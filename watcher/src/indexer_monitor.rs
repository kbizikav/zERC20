// SPDX-License-Identifier: BUSL-1.1

use std::collections::HashMap;

use anyhow::{Context, Result};
use api_types::indexer::TokenStatusResponse;
use client_common::{
    contracts::{verifier::VerifierContract, z_erc20::ZErc20Contract},
    tokens::{TokenEntry, load_tokens_from_path},
};
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::{IndexerConfig, TokenConfig};

/// Per-chain state for staleness detection.
#[derive(Debug, Clone, Default)]
struct ChainState {
    prev_tree_synced: Option<u64>,
    tree_stale_count: u32,
    prev_proved_index: Option<u64>,
    proved_stale_count: u32,
}

pub struct IndexerMonitor {
    config: IndexerConfig,
    tokens: Vec<TokenConfig>,
    client: reqwest::Client,
    /// Keyed by `{token_name}:{chain_id}`.
    state: HashMap<String, ChainState>,
}

impl IndexerMonitor {
    pub fn new(config: IndexerConfig, tokens: Vec<TokenConfig>) -> Self {
        Self {
            config,
            tokens,
            client: reqwest::Client::new(),
            state: HashMap::new(),
        }
    }

    pub async fn check(&mut self) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let threshold = self.config.stale_threshold_cycles;

        // Field-level borrows: &self.tokens and &self.client are immutable,
        // while &mut self.state is mutable — no clone needed.
        for token in &self.tokens {
            let base_url = match token.indexer_url.as_ref() {
                Some(u) => u.trim_end_matches('/'),
                None => continue,
            };

            // 1. Healthz check
            if let Some(alert) = check_healthz(&self.client, &token.name, base_url).await {
                alerts.push(alert);
                continue;
            }

            // 2. Fetch per-chain statuses from indexer
            let statuses = match fetch_statuses(&self.client, &token.name, base_url).await {
                Ok(s) => s,
                Err(err) => {
                    error!("status fetch failed for {}: {:?}", token.name, err);
                    alerts.push(Alert {
                        severity: Severity::Critical,
                        domain: "indexer".to_string(),
                        title: format!("Status fetch failed: {}", token.name),
                        description: format!(
                            "Failed to fetch status for **{}**: {}",
                            token.name, err
                        ),
                        fields: vec![],
                    });
                    continue;
                }
            };

            // 3. Load crosschain config for on-chain contract access
            let token_entries = load_token_entries(token);

            // 4. Per-chain staleness checks
            for status in &statuses {
                let tree_synced = match status.tree_synced_index {
                    Some(idx) => idx,
                    None => continue,
                };

                let entry = token_entries.iter().find(|e| e.chain_id == status.chain_id);
                let chain_label = entry.map(|e| e.label.as_str()).unwrap_or("unknown");

                let (onchain_index, proved_index) =
                    fetch_chain_indices(&token.name, chain_label, entry).await;

                let key = format!("{}:{}", token.name, status.chain_id);
                let state = self.state.entry(key).or_default();

                if let Some(alert) = check_tree_staleness(
                    &token.name,
                    chain_label,
                    tree_synced,
                    onchain_index,
                    state,
                    threshold,
                ) {
                    alerts.push(alert);
                }

                if let Some(alert) = check_proved_staleness(
                    &token.name,
                    chain_label,
                    tree_synced,
                    proved_index,
                    state,
                    threshold,
                ) {
                    alerts.push(alert);
                }
            }
        }

        alerts
    }
}

/// Check indexer healthz endpoint. Returns `Some(Alert)` on failure.
async fn check_healthz(
    client: &reqwest::Client,
    token_name: &str,
    base_url: &str,
) -> Option<Alert> {
    let healthz_url = format!("{}/healthz", base_url);
    match client.get(&healthz_url).send().await {
        Ok(resp) if resp.status().is_success() => None,
        Ok(resp) => {
            error!(
                "healthz failed for {} ({}): HTTP {}",
                token_name,
                healthz_url,
                resp.status()
            );
            Some(Alert {
                severity: Severity::Critical,
                domain: "indexer".to_string(),
                title: format!("Indexer unhealthy: {}", token_name),
                description: format!(
                    "Healthz endpoint for **{}** returned HTTP {}.",
                    token_name,
                    resp.status()
                ),
                fields: vec![AlertField {
                    name: "URL".to_string(),
                    value: healthz_url,
                    inline: false,
                }],
            })
        }
        Err(err) => {
            error!(
                "healthz unreachable for {} ({}): {:?}",
                token_name, healthz_url, err
            );
            Some(Alert {
                severity: Severity::Critical,
                domain: "indexer".to_string(),
                title: format!("Indexer unreachable: {}", token_name),
                description: format!(
                    "Failed to reach healthz endpoint for **{}**: {}",
                    token_name, err
                ),
                fields: vec![AlertField {
                    name: "URL".to_string(),
                    value: healthz_url,
                    inline: false,
                }],
            })
        }
    }
}

/// Fetch per-chain statuses from the /status endpoint.
async fn fetch_statuses(
    client: &reqwest::Client,
    name: &str,
    base_url: &str,
) -> Result<Vec<TokenStatusResponse>> {
    let url = format!("{}/status", base_url);
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {} for {}", url, name))?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}", resp.status(), url);
    }

    resp.json()
        .await
        .with_context(|| format!("deserialize status for {}", name))
}

/// Load token entries from crosschain config, returning empty vec on failure.
fn load_token_entries(token: &TokenConfig) -> Vec<TokenEntry> {
    match &token.crosschain_config_path {
        Some(path) => match load_tokens_from_path(path) {
            Ok(tf) => tf.tokens,
            Err(err) => {
                error!(
                    "failed to load crosschain config for {}: {:?}",
                    token.name, err
                );
                vec![]
            }
        },
        None => vec![],
    }
}

/// Fetch on-chain index and proved index for a single chain.
async fn fetch_chain_indices(
    token_name: &str,
    chain_label: &str,
    entry: Option<&TokenEntry>,
) -> (Option<u64>, Option<u64>) {
    let entry = match entry {
        Some(e) => e,
        None => return (None, None),
    };

    let onchain = match fetch_onchain_index(entry).await {
        Ok(idx) => Some(idx),
        Err(err) => {
            error!(
                "failed to fetch on-chain index for {} ({}): {:?}",
                token_name, chain_label, err
            );
            None
        }
    };

    let proved = match fetch_proved_index(entry).await {
        Ok(idx) => Some(idx),
        Err(err) => {
            error!(
                "failed to fetch proved index for {} ({}): {:?}",
                token_name, chain_label, err
            );
            None
        }
    };

    (onchain, proved)
}

/// Update tree_synced staleness state and return alert if stale threshold exceeded.
fn check_tree_staleness(
    token_name: &str,
    chain_label: &str,
    tree_synced: u64,
    onchain_index: Option<u64>,
    state: &mut ChainState,
    threshold: u32,
) -> Option<Alert> {
    if let Some(prev) = state.prev_tree_synced {
        if tree_synced == prev {
            state.tree_stale_count += 1;
        } else {
            state.tree_stale_count = 0;
        }
    }
    state.prev_tree_synced = Some(tree_synced);

    if state.tree_stale_count < threshold {
        return None;
    }

    // Only alert when on-chain is actually ahead (i.e. there IS work to sync)
    let onchain = onchain_index.filter(|&o| o > tree_synced)?;

    info!(
        "TREE STALE: {} ({}) — tree_synced={} unchanged for {} cycles, onchain={}",
        token_name, chain_label, tree_synced, state.tree_stale_count, onchain
    );

    Some(Alert {
        severity: Severity::Warning,
        domain: "indexer".to_string(),
        title: format!("tree_synced stale: {} ({})", token_name, chain_label),
        description: format!(
            "**{}** ({}): `tree_synced_index` ({}) has not progressed for **{}** cycles while on-chain index is {}.",
            token_name, chain_label, tree_synced, state.tree_stale_count, onchain
        ),
        fields: vec![
            AlertField {
                name: "on-chain".to_string(),
                value: onchain.to_string(),
                inline: true,
            },
            AlertField {
                name: "tree_synced".to_string(),
                value: tree_synced.to_string(),
                inline: true,
            },
            AlertField {
                name: "Stale cycles".to_string(),
                value: state.tree_stale_count.to_string(),
                inline: true,
            },
        ],
    })
}

/// Update proved index staleness state and return alert if stale threshold exceeded.
fn check_proved_staleness(
    token_name: &str,
    chain_label: &str,
    tree_synced: u64,
    proved_index: Option<u64>,
    state: &mut ChainState,
    threshold: u32,
) -> Option<Alert> {
    let proved = proved_index?;

    if let Some(prev_proved) = state.prev_proved_index {
        if proved == prev_proved && tree_synced > proved {
            state.proved_stale_count += 1;
        } else {
            state.proved_stale_count = 0;
        }
    }
    state.prev_proved_index = Some(proved);

    if state.proved_stale_count < threshold {
        return None;
    }

    info!(
        "PROVED STALE: {} ({}) — proved={} unchanged for {} cycles, tree_synced={}",
        token_name, chain_label, proved, state.proved_stale_count, tree_synced
    );

    Some(Alert {
        severity: Severity::Warning,
        domain: "indexer".to_string(),
        title: format!("Proved index stale: {} ({})", token_name, chain_label),
        description: format!(
            "**{}** ({}): `latestProvedIndex` ({}) has not progressed for **{}** cycles while `tree_synced_index` is {}.",
            token_name, chain_label, proved, state.proved_stale_count, tree_synced
        ),
        fields: vec![
            AlertField {
                name: "tree_synced".to_string(),
                value: tree_synced.to_string(),
                inline: true,
            },
            AlertField {
                name: "proved".to_string(),
                value: proved.to_string(),
                inline: true,
            },
            AlertField {
                name: "Stale cycles".to_string(),
                value: state.proved_stale_count.to_string(),
                inline: true,
            },
        ],
    })
}

/// Fetch on-chain tree index from the zERC20 token contract.
async fn fetch_onchain_index(entry: &TokenEntry) -> Result<u64> {
    let provider = entry.provider()?;
    let contract = ZErc20Contract::new(provider, entry.token_address);
    contract.index().await.context("ZErc20.index() call failed")
}

/// Fetch latestProvedIndex from the verifier contract.
async fn fetch_proved_index(entry: &TokenEntry) -> Result<u64> {
    let provider = entry.provider()?;
    let verifier = VerifierContract::new(provider, entry.verifier_address);
    verifier
        .latest_proved_index()
        .await
        .context("Verifier.latestProvedIndex() call failed")
}
