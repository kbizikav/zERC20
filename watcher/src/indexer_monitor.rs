use std::collections::HashMap;

use anyhow::{Context, Result};
use client_common::{
    contracts::verifier::VerifierContract,
    tokens::{TokensFile, load_tokens_from_path},
};
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::{IndexerConfig, TokenConfig};

/// Per-token state for staleness detection.
#[derive(Debug, Clone, Default)]
struct TokenState {
    prev_tree_synced: Option<u64>,
    tree_stale_count: u32,
    prev_proved_index: Option<u64>,
    proved_stale_count: u32,
}

pub struct IndexerMonitor {
    config: IndexerConfig,
    tokens: Vec<TokenConfig>,
    client: reqwest::Client,
    state: HashMap<String, TokenState>,
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

            // 2. Fetch status
            let status_url = format!("{}/status", base_url);
            let tree_synced_index = match self.fetch_tree_synced(&token.name, &status_url).await {
                Ok(Some(idx)) => idx,
                Ok(None) => continue,
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

            // 3. Fetch latestProvedIndex from verifier
            let proved_index = match &token.crosschain_config_path {
                Some(path) => match fetch_min_proved_index(path).await {
                    Ok(Some(idx)) => Some(idx),
                    Ok(None) => None,
                    Err(err) => {
                        error!("failed to fetch proved index for {}: {:?}", token.name, err);
                        None
                    }
                },
                None => None,
            };

            // 4. Staleness checks
            let state = self.state.entry(token.name.clone()).or_default();

            // tree_synced staleness
            if let Some(prev) = state.prev_tree_synced {
                if tree_synced_index == prev {
                    state.tree_stale_count += 1;
                } else {
                    state.tree_stale_count = 0;
                }
            }
            state.prev_tree_synced = Some(tree_synced_index);

            if state.tree_stale_count >= threshold {
                let has_unproved = proved_index.map(|p| p < tree_synced_index).unwrap_or(false);
                if has_unproved {
                    info!(
                        "TREE STALE: {} — tree_synced_index={} unchanged for {} cycles, proved={}",
                        token.name,
                        tree_synced_index,
                        state.tree_stale_count,
                        proved_index.unwrap()
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "indexer".to_string(),
                        title: format!("tree_synced stale: {}", token.name),
                        description: format!(
                            "**{}**: `tree_synced_index` ({}) has not progressed for **{}** cycles while proved index ({}) is behind.",
                            token.name, tree_synced_index, state.tree_stale_count, proved_index.unwrap()
                        ),
                        fields: vec![
                            AlertField {
                                name: "tree_synced".to_string(),
                                value: tree_synced_index.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "proved".to_string(),
                                value: proved_index.unwrap().to_string(),
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

            // proved index staleness
            if let Some(proved) = proved_index {
                if let Some(prev_proved) = state.prev_proved_index {
                    if proved == prev_proved && proved < tree_synced_index {
                        state.proved_stale_count += 1;
                    } else {
                        state.proved_stale_count = 0;
                    }
                }
                state.prev_proved_index = Some(proved);

                if state.proved_stale_count >= threshold {
                    info!(
                        "PROVED STALE: {} — latestProvedIndex={} unchanged for {} cycles, tree_synced={}",
                        token.name, proved, state.proved_stale_count, tree_synced_index
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "indexer".to_string(),
                        title: format!("Proved index stale: {}", token.name),
                        description: format!(
                            "**{}**: `latestProvedIndex` ({}) has not progressed for **{}** cycles while `tree_synced_index` is {}.",
                            token.name, proved, state.proved_stale_count, tree_synced_index
                        ),
                        fields: vec![
                            AlertField {
                                name: "proved".to_string(),
                                value: proved.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "tree_synced".to_string(),
                                value: tree_synced_index.to_string(),
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

        alerts
    }

    /// Fetch tree_synced_index from the /status endpoint.
    async fn fetch_tree_synced(&self, name: &str, url: &str) -> Result<Option<u64>> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {} for {}", url, name))?;

        if !resp.status().is_success() {
            anyhow::bail!("HTTP {} from {}", resp.status(), url);
        }

        let statuses: Vec<api_types::indexer::TokenStatusResponse> = resp
            .json()
            .await
            .with_context(|| format!("deserialize status for {}", name))?;

        Ok(statuses.first().and_then(|s| s.tree_synced_index))
    }
}

/// Load verifier contracts from crosschain config and return the minimum latestProvedIndex.
async fn fetch_min_proved_index(config_path: &str) -> Result<Option<u64>> {
    let tokens_file: TokensFile =
        load_tokens_from_path(config_path).with_context(|| format!("loading {}", config_path))?;

    let mut min_proved: Option<u64> = None;

    for token in &tokens_file.tokens {
        let provider = match token.provider() {
            Ok(p) => p,
            Err(err) => {
                error!("failed to create provider for '{}': {:?}", token.label, err);
                continue;
            }
        };

        let verifier = VerifierContract::new(provider, token.verifier_address);
        match verifier.latest_proved_index().await {
            Ok(idx) => {
                min_proved = Some(min_proved.map_or(idx, |m: u64| m.min(idx)));
            }
            Err(err) => {
                error!(
                    "failed to read latestProvedIndex for '{}': {:?}",
                    token.label, err
                );
            }
        }
    }

    Ok(min_proved)
}
