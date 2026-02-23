use std::collections::HashMap;

use anyhow::{Context, Result};
use api_types::indexer::TokenStatusResponse;
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::IndexerConfig;

/// Snapshot of index values for a single token, used for stale detection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenSnapshot {
    events_synced: Option<u64>,
    tree_synced: Option<u64>,
    ivc_generated: Option<u64>,
    onchain_proved: Option<u64>,
}

impl From<&TokenStatusResponse> for TokenSnapshot {
    fn from(status: &TokenStatusResponse) -> Self {
        Self {
            events_synced: status.events_synced_index,
            tree_synced: status.tree_synced_index,
            ivc_generated: status.ivc_generated_index,
            onchain_proved: status.onchain_proved_index,
        }
    }
}

/// Key used to look up per-token state.
fn token_key(status: &TokenStatusResponse) -> String {
    format!("{}:{}", status.chain_id, status.token_address)
}

pub struct IndexerMonitor {
    config: IndexerConfig,
    client: reqwest::Client,
    /// Previous snapshots keyed by `chain_id:token_address`.
    prev_snapshots: HashMap<String, TokenSnapshot>,
    /// How many consecutive cycles each token has been unchanged.
    stale_counts: HashMap<String, u32>,
}

impl IndexerMonitor {
    pub fn new(config: IndexerConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            prev_snapshots: HashMap::new(),
            stale_counts: HashMap::new(),
        }
    }

    pub async fn check(&mut self) -> Vec<Alert> {
        match self.fetch_status().await {
            Ok(statuses) => {
                let mut alerts = Vec::new();
                alerts.extend(self.check_index_gaps(&statuses));
                alerts.extend(self.check_staleness(&statuses));
                alerts
            }
            Err(err) => {
                error!("indexer status fetch failed: {:?}", err);
                vec![Alert {
                    severity: Severity::Critical,
                    domain: "indexer".to_string(),
                    title: "Indexer unreachable".to_string(),
                    description: format!(
                        "Failed to reach indexer at `{}`: {}",
                        self.config.status_url, err
                    ),
                    fields: vec![],
                }]
            }
        }
    }

    async fn fetch_status(&self) -> Result<Vec<TokenStatusResponse>> {
        let resp = self
            .client
            .get(&self.config.status_url)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to GET indexer status from {}",
                    self.config.status_url
                )
            })?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "indexer status returned HTTP {}",
                resp.status()
            );
        }

        resp.json::<Vec<TokenStatusResponse>>()
            .await
            .context("failed to deserialize indexer status response")
    }

    fn check_index_gaps(&self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let max_gap = self.config.max_index_gap;

        for status in statuses {
            // Define the pipeline stages in order
            let stages: Vec<(&str, Option<u64>)> = vec![
                ("events_synced", status.events_synced_index),
                ("tree_synced", status.tree_synced_index),
                ("ivc_generated", status.ivc_generated_index),
                ("onchain_proved", status.onchain_proved_index),
            ];

            for window in stages.windows(2) {
                let (ahead_name, ahead_val) = &window[0];
                let (behind_name, behind_val) = &window[1];

                if let (Some(ahead), Some(behind)) = (ahead_val, behind_val) {
                    if ahead.saturating_sub(*behind) > max_gap {
                        let label = &status.label;
                        info!(
                            "INDEX GAP: {} — {} ({}) vs {} ({}), gap={}",
                            label,
                            ahead_name,
                            ahead,
                            behind_name,
                            behind,
                            ahead - behind
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "indexer".to_string(),
                            title: format!(
                                "Index gap: {} ({} → {})",
                                label, ahead_name, behind_name
                            ),
                            description: format!(
                                "Token **{}** (chain {}) has a gap of **{}** between `{}` ({}) and `{}` ({}).",
                                label,
                                status.chain_id,
                                ahead - behind,
                                ahead_name,
                                ahead,
                                behind_name,
                                behind,
                            ),
                            fields: vec![
                                AlertField {
                                    name: "Token".to_string(),
                                    value: label.clone(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Chain".to_string(),
                                    value: status.chain_id.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: ahead_name.to_string(),
                                    value: ahead.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: behind_name.to_string(),
                                    value: behind.to_string(),
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

    fn check_staleness(&mut self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let threshold = self.config.stale_threshold_cycles;

        for status in statuses {
            let key = token_key(status);
            let current = TokenSnapshot::from(status);

            let is_stale = match self.prev_snapshots.get(&key) {
                Some(prev) => prev == &current,
                None => false,
            };

            if is_stale {
                let count = self.stale_counts.entry(key.clone()).or_insert(0);
                *count += 1;

                if *count >= threshold {
                    info!(
                        "STALE: {} — no progress for {} consecutive cycles",
                        status.label, count
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "indexer".to_string(),
                        title: format!("Indexer stale: {}", status.label),
                        description: format!(
                            "Token **{}** (chain {}) has shown no indexing progress for **{}** consecutive cycles.",
                            status.label, status.chain_id, count
                        ),
                        fields: vec![
                            AlertField {
                                name: "Token".to_string(),
                                value: status.label.clone(),
                                inline: true,
                            },
                            AlertField {
                                name: "Chain".to_string(),
                                value: status.chain_id.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "Stale cycles".to_string(),
                                value: count.to_string(),
                                inline: true,
                            },
                        ],
                    });
                }
            } else {
                self.stale_counts.insert(key.clone(), 0);
            }

            self.prev_snapshots.insert(key, current);
        }

        alerts
    }
}
