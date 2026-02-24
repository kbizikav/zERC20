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

        for token in &self.tokens.clone() {
            let base_url = match token.indexer_url.as_ref() {
                Some(u) => u.trim_end_matches('/'),
                None => continue,
            };

            // 1. Healthz check
            let healthz_url = format!("{}/healthz", base_url);
            match self.client.get(&healthz_url).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    error!(
                        "healthz failed for {} ({}): HTTP {}",
                        token.name,
                        healthz_url,
                        resp.status()
                    );
                    alerts.push(Alert {
                        severity: Severity::Critical,
                        domain: "indexer".to_string(),
                        title: format!("Indexer unhealthy: {}", token.name),
                        description: format!(
                            "Healthz endpoint for **{}** returned HTTP {}.",
                            token.name,
                            resp.status()
                        ),
                        fields: vec![AlertField {
                            name: "URL".to_string(),
                            value: healthz_url,
                            inline: false,
                        }],
                    });
                    continue;
                }
                Err(err) => {
                    error!(
                        "healthz unreachable for {} ({}): {:?}",
                        token.name, healthz_url, err
                    );
                    alerts.push(Alert {
                        severity: Severity::Critical,
                        domain: "indexer".to_string(),
                        title: format!("Indexer unreachable: {}", token.name),
                        description: format!(
                            "Failed to reach healthz endpoint for **{}**: {}",
                            token.name, err
                        ),
                        fields: vec![AlertField {
                            name: "URL".to_string(),
                            value: healthz_url,
                            inline: false,
                        }],
                    });
                    continue;
                }
            }

            // 2. Fetch per-chain statuses from indexer
            let status_url = format!("{}/status", base_url);
            let statuses = match self.fetch_statuses(&token.name, &status_url).await {
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
            let token_entries: Vec<TokenEntry> = match &token.crosschain_config_path {
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
            };

            // 4. Per-chain staleness checks
            for status in &statuses {
                let tree_synced = match status.tree_synced_index {
                    Some(idx) => idx,
                    None => continue,
                };

                let entry = token_entries.iter().find(|e| e.chain_id == status.chain_id);
                let chain_label = entry.map(|e| e.label.as_str()).unwrap_or("unknown");

                // Fetch on-chain index from token contract
                let onchain_index = match entry {
                    Some(e) => match fetch_onchain_index(e).await {
                        Ok(idx) => Some(idx),
                        Err(err) => {
                            error!(
                                "failed to fetch on-chain index for {} ({}): {:?}",
                                token.name, chain_label, err
                            );
                            None
                        }
                    },
                    None => None,
                };

                // Fetch proved index from verifier contract
                let proved_index = match entry {
                    Some(e) => match fetch_proved_index(e).await {
                        Ok(idx) => Some(idx),
                        Err(err) => {
                            error!(
                                "failed to fetch proved index for {} ({}): {:?}",
                                token.name, chain_label, err
                            );
                            None
                        }
                    },
                    None => None,
                };

                let key = format!("{}:{}", token.name, status.chain_id);
                let state = self.state.entry(key).or_default();

                // tree_synced staleness: hasn't advanced while on-chain index is ahead
                if let Some(prev) = state.prev_tree_synced {
                    if tree_synced == prev {
                        state.tree_stale_count += 1;
                    } else {
                        state.tree_stale_count = 0;
                    }
                }
                state.prev_tree_synced = Some(tree_synced);

                if state.tree_stale_count >= threshold {
                    let onchain_ahead = onchain_index.map(|o| o > tree_synced).unwrap_or(false);
                    if onchain_ahead {
                        info!(
                            "TREE STALE: {} ({}) — tree_synced={} unchanged for {} cycles, onchain={}",
                            token.name,
                            chain_label,
                            tree_synced,
                            state.tree_stale_count,
                            onchain_index.unwrap()
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "indexer".to_string(),
                            title: format!(
                                "tree_synced stale: {} ({})",
                                token.name, chain_label
                            ),
                            description: format!(
                                "**{}** ({}): `tree_synced_index` ({}) has not progressed for **{}** cycles while on-chain index is {}.",
                                token.name, chain_label, tree_synced, state.tree_stale_count,
                                onchain_index.unwrap()
                            ),
                            fields: vec![
                                AlertField {
                                    name: "on-chain".to_string(),
                                    value: onchain_index.unwrap().to_string(),
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
                        });
                    }
                }

                // proved staleness: hasn't advanced while tree_synced is ahead
                if let Some(proved) = proved_index {
                    if let Some(prev_proved) = state.prev_proved_index {
                        if proved == prev_proved && tree_synced > proved {
                            state.proved_stale_count += 1;
                        } else {
                            state.proved_stale_count = 0;
                        }
                    }
                    state.prev_proved_index = Some(proved);

                    if state.proved_stale_count >= threshold {
                        info!(
                            "PROVED STALE: {} ({}) — proved={} unchanged for {} cycles, tree_synced={}",
                            token.name, chain_label, proved, state.proved_stale_count, tree_synced
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "indexer".to_string(),
                            title: format!(
                                "Proved index stale: {} ({})",
                                token.name, chain_label
                            ),
                            description: format!(
                                "**{}** ({}): `latestProvedIndex` ({}) has not progressed for **{}** cycles while `tree_synced_index` is {}.",
                                token.name, chain_label, proved, state.proved_stale_count, tree_synced
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
                        });
                    }
                }
            }
        }

        alerts
    }

    /// Fetch per-chain statuses from the /status endpoint.
    async fn fetch_statuses(&self, name: &str, url: &str) -> Result<Vec<TokenStatusResponse>> {
        let resp = self
            .client
            .get(url)
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
