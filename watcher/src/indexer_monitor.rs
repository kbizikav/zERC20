use std::collections::HashMap;

use anyhow::{Context, Result};
use api_types::indexer::TokenStatusResponse;
use log::{error, info, warn};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::IndexerConfig;

/// Snapshot of index values for a single token, used for stale and regression detection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenSnapshot {
    events_synced: Option<u64>,
    tree_synced: Option<u64>,
    ivc_generated: Option<u64>,
    onchain_reserved: Option<u64>,
    onchain_proved: Option<u64>,
}

impl TokenSnapshot {
    fn from_status(status: &TokenStatusResponse) -> Self {
        Self {
            events_synced: status.events_synced_index,
            tree_synced: status.tree_synced_index,
            ivc_generated: status.ivc_generated_index,
            onchain_reserved: status.onchain_reserved_index,
            onchain_proved: status.onchain_proved_index,
        }
    }

    /// Returns ordered pipeline stages for gap checking.
    fn stages(&self) -> Vec<(&'static str, Option<u64>)> {
        vec![
            ("events_synced", self.events_synced),
            ("tree_synced", self.tree_synced),
            ("ivc_generated", self.ivc_generated),
            ("onchain_reserved", self.onchain_reserved),
            ("onchain_proved", self.onchain_proved),
        ]
    }
}

/// Per-stage stale cycle counter.
#[derive(Debug, Clone, Default)]
struct StaleCounts {
    events_synced: u32,
    tree_synced: u32,
    ivc_generated: u32,
    onchain_reserved: u32,
    onchain_proved: u32,
}

impl StaleCounts {
    /// Returns (stage_name, count) pairs for stages that have reached the threshold.
    fn stale_stages(&self, threshold: u32) -> Vec<(&'static str, u32)> {
        let all = [
            ("events_synced", self.events_synced),
            ("tree_synced", self.tree_synced),
            ("ivc_generated", self.ivc_generated),
            ("onchain_reserved", self.onchain_reserved),
            ("onchain_proved", self.onchain_proved),
        ];
        all.into_iter()
            .filter(|(_, count)| *count >= threshold)
            .collect()
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
    /// Per-stage stale cycle counters keyed by `chain_id:token_address`.
    stale_counts: HashMap<String, StaleCounts>,
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
                alerts.extend(self.check_none_after_data(&statuses));
                alerts.extend(self.check_regression(&statuses));
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
            anyhow::bail!("indexer status returned HTTP {}", resp.status());
        }

        resp.json::<Vec<TokenStatusResponse>>()
            .await
            .context("failed to deserialize indexer status response")
    }

    /// Check that adjacent pipeline stages don't have excessive gaps.
    /// Full pipeline: events_synced → tree_synced → ivc_generated → onchain_reserved → onchain_proved
    fn check_index_gaps(&self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let max_gap = self.config.max_index_gap;

        for status in statuses {
            let snapshot = TokenSnapshot::from_status(status);
            let stages = snapshot.stages();

            for window in stages.windows(2) {
                let (ahead_name, ahead_val) = &window[0];
                let (behind_name, behind_val) = &window[1];

                if let (Some(ahead), Some(behind)) = (ahead_val, behind_val) {
                    let gap = ahead.saturating_sub(*behind);
                    if gap > max_gap {
                        let label = &status.label;
                        info!(
                            "INDEX GAP: {} — {} ({}) vs {} ({}), gap={}",
                            label, ahead_name, ahead, behind_name, behind, gap
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
                                label, status.chain_id, gap, ahead_name, ahead, behind_name, behind,
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

    /// Detect when an upstream stage has data but a downstream stage is None.
    /// e.g. events_synced=500, tree_synced=None → TreeIngestion job may be broken.
    /// onchain_reserved/onchain_proved returning None could mean RPC is unreachable.
    fn check_none_after_data(&self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();

        for status in statuses {
            let snapshot = TokenSnapshot::from_status(status);
            let stages = snapshot.stages();

            // Track the last stage that had data
            let mut last_with_data: Option<(&str, u64)> = None;

            for (name, value) in &stages {
                match value {
                    Some(v) => {
                        last_with_data = Some((name, *v));
                    }
                    None => {
                        if let Some((upstream_name, upstream_val)) = last_with_data {
                            // An upstream stage has data but this downstream stage is None
                            warn!(
                                "NONE AFTER DATA: {} — {} has data ({}) but {} is None",
                                status.label, upstream_name, upstream_val, name
                            );
                            alerts.push(Alert {
                                severity: Severity::Warning,
                                domain: "indexer".to_string(),
                                title: format!("Stage unavailable: {} ({})", status.label, name),
                                description: format!(
                                    "Token **{}** (chain {}): `{}` has data ({}) but downstream stage `{}` is unavailable (None).",
                                    status.label, status.chain_id, upstream_name, upstream_val, name,
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
                                        name: format!("{} (has data)", upstream_name),
                                        value: upstream_val.to_string(),
                                        inline: true,
                                    },
                                    AlertField {
                                        name: format!("{} (missing)", name),
                                        value: "None".to_string(),
                                        inline: true,
                                    },
                                ],
                            });
                            // Only report the first None gap per token to avoid noise
                            break;
                        }
                    }
                }
            }
        }

        alerts
    }

    /// Detect index regression (value decreased since last check).
    /// This can indicate a reorg (events_synced) or data corruption.
    fn check_regression(&self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();

        for status in statuses {
            let key = token_key(status);
            let prev = match self.prev_snapshots.get(&key) {
                Some(p) => p,
                None => continue,
            };

            let current = TokenSnapshot::from_status(status);
            let prev_stages = prev.stages();
            let curr_stages = current.stages();

            for ((name, prev_val), (_, curr_val)) in prev_stages.iter().zip(curr_stages.iter()) {
                if let (Some(prev_v), Some(curr_v)) = (prev_val, curr_val) {
                    if curr_v < prev_v {
                        warn!(
                            "REGRESSION: {} — {} decreased from {} to {}",
                            status.label, name, prev_v, curr_v
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "indexer".to_string(),
                            title: format!("Index regression: {} ({})", status.label, name),
                            description: format!(
                                "Token **{}** (chain {}): `{}` decreased from **{}** to **{}**. Possible reorg or data issue.",
                                status.label, status.chain_id, name, prev_v, curr_v,
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
                                    name: "Previous".to_string(),
                                    value: prev_v.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Current".to_string(),
                                    value: curr_v.to_string(),
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

    /// Per-stage staleness detection.
    /// Identifies which specific stage is stuck rather than treating all stages as one unit.
    /// A stage is considered stale only if an upstream stage is progressing but this stage is not.
    fn check_staleness(&mut self, statuses: &[TokenStatusResponse]) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let threshold = self.config.stale_threshold_cycles;

        for status in statuses {
            let key = token_key(status);
            let current = TokenSnapshot::from_status(status);

            if let Some(prev) = self.prev_snapshots.get(&key) {
                let counts = self.stale_counts.entry(key.clone()).or_default();

                // Check each stage: increment stale count if unchanged, reset if progressed
                update_stage_count(
                    &mut counts.events_synced,
                    prev.events_synced,
                    current.events_synced,
                );
                update_stage_count(
                    &mut counts.tree_synced,
                    prev.tree_synced,
                    current.tree_synced,
                );
                update_stage_count(
                    &mut counts.ivc_generated,
                    prev.ivc_generated,
                    current.ivc_generated,
                );
                update_stage_count(
                    &mut counts.onchain_reserved,
                    prev.onchain_reserved,
                    current.onchain_reserved,
                );
                update_stage_count(
                    &mut counts.onchain_proved,
                    prev.onchain_proved,
                    current.onchain_proved,
                );

                // Only alert for stages that are stale while an upstream stage has data ahead.
                // This avoids alerting when the entire pipeline is idle (no new events).
                let current_stages = current.stages();
                let stale_stages = counts.stale_stages(threshold);

                for (stale_name, stale_count) in &stale_stages {
                    // Find this stage's position and check if any upstream stage has more data
                    let stage_pos = current_stages.iter().position(|(n, _)| n == stale_name);
                    let has_upstream_ahead = stage_pos.is_some_and(|pos| {
                        if pos == 0 {
                            // events_synced is the first stage; if it's stale, it may just
                            // mean no new on-chain events have occurred — not necessarily a
                            // problem.
                            return false;
                        }
                        let upstream_val = current_stages[pos - 1].1;
                        let this_val = current_stages[pos].1;
                        match (upstream_val, this_val) {
                            (Some(up), Some(this)) => up > this,
                            (Some(_), None) => true,
                            _ => false,
                        }
                    });

                    if has_upstream_ahead {
                        info!(
                            "STAGE STALE: {} — {} stuck for {} cycles while upstream has more data",
                            status.label, stale_name, stale_count
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "indexer".to_string(),
                            title: format!("Stage stale: {} ({})", status.label, stale_name),
                            description: format!(
                                "Token **{}** (chain {}): stage `{}` has not progressed for **{}** consecutive cycles while upstream data is available.",
                                status.label, status.chain_id, stale_name, stale_count,
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
                                    name: "Stuck stage".to_string(),
                                    value: stale_name.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Stale cycles".to_string(),
                                    value: stale_count.to_string(),
                                    inline: true,
                                },
                            ],
                        });
                    }
                }
            }

            self.prev_snapshots.insert(key, current);
        }

        alerts
    }
}

/// Update a per-stage stale counter: increment if value unchanged, reset if progressed.
fn update_stage_count(count: &mut u32, prev: Option<u64>, current: Option<u64>) {
    match (prev, current) {
        (Some(p), Some(c)) if c == p => *count += 1,
        (None, None) => *count += 1,
        _ => *count = 0,
    }
}
