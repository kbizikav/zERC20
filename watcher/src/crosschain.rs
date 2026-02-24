use alloy::primitives::U256;
use anyhow::{Context, Result};
use client_common::{
    contracts::{hub::HubContract, verifier::VerifierContract},
    tokens::{TokensFile, load_tokens_from_path},
};
use log::{error, info};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::TokenConfig;

/// Lookback window for hub event queries (~7 days on Ethereum).
const HUB_EVENT_LOOKBACK_BLOCKS: u64 = 50_000;

/// Maximum block range per `eth_getLogs` request (RPC provider limit).
const HUB_EVENT_CHUNK_SIZE: u64 = 10_000;

pub async fn check_crosschain(tokens: &[TokenConfig], root_delay_threshold: u64) -> Vec<Alert> {
    let mut alerts = Vec::new();

    for token in tokens {
        let path = match token.crosschain_config_path.as_ref() {
            Some(p) => p,
            None => continue,
        };
        match check_single_config(path, root_delay_threshold).await {
            Ok(mut a) => alerts.append(&mut a),
            Err(err) => {
                error!("crosschain check failed for {}: {:?}", path, err);
                alerts.push(Alert {
                    severity: Severity::Critical,
                    domain: "crosschain".to_string(),
                    title: format!("Config load failed: {}", path),
                    description: format!(
                        "Failed to load or process token config `{}`: {}",
                        path, err
                    ),
                    fields: vec![],
                });
            }
        }
    }

    alerts
}

async fn check_single_config(path: &str, root_delay_threshold: u64) -> Result<Vec<Alert>> {
    let tokens_file: TokensFile =
        load_tokens_from_path(path).with_context(|| format!("loading {}", path))?;

    let hub_entry = tokens_file
        .hub
        .as_ref()
        .context("token config has no hub entry")?;

    let hub_provider = hub_entry.provider()?;
    let hub = HubContract::new(hub_provider, hub_entry.hub_address);

    let hub_agg_seq = hub.agg_seq().await.context("failed to read hub agg_seq")?;

    let hub_root = hub
        .current_aggregation_root()
        .await
        .context("failed to read hub current_aggregation_root")?;

    if hub_root == U256::ZERO {
        info!("hub aggregation root is zero, skipping crosschain checks for {path}");
        return Ok(Vec::new());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut alerts = Vec::new();

    // Cache: aggSeq → Option<timestamp>. Queried once per distinct aggSeq,
    // shared across all verifiers in this token config.
    let mut ts_cache: HashMap<u64, Option<u64>> = HashMap::new();

    for token in &tokens_file.tokens {
        let label = &token.label;
        let provider = match token.provider() {
            Ok(p) => p,
            Err(err) => {
                error!("failed to create provider for '{}': {:?}", label, err);
                continue;
            }
        };

        let verifier = VerifierContract::new(provider, token.verifier_address);

        let v_agg_seq = match verifier.latest_agg_seq().await {
            Ok(seq) => seq,
            Err(err) => {
                error!("failed to read latest_agg_seq for '{}': {:?}", label, err);
                continue;
            }
        };

        if v_agg_seq == 0 {
            info!("ROOT NOT SYNCED: {} — verifier aggSeq is 0", label);
            alerts.push(Alert {
                severity: Severity::Warning,
                domain: "crosschain".to_string(),
                title: format!("Root not synced: {}", label),
                description: format!(
                    "Verifier **{}** (chain {}) has not received any aggregation root yet (aggSeq=0).",
                    label, token.chain_id,
                ),
                fields: vec![
                    AlertField {
                        name: "Token".to_string(),
                        value: label.clone(),
                        inline: true,
                    },
                    AlertField {
                        name: "Chain".to_string(),
                        value: token.chain_id.to_string(),
                        inline: true,
                    },
                ],
            });
            continue;
        }

        let v_root = match verifier.global_transfer_root(v_agg_seq).await {
            Ok(r) => r,
            Err(err) => {
                error!(
                    "failed to read global_transfer_root for '{}': {:?}",
                    label, err
                );
                continue;
            }
        };

        // Critical: same aggSeq but different root → protocol-level inconsistency
        if v_agg_seq == hub_agg_seq && v_root != hub_root && v_root != U256::ZERO {
            info!(
                "ROOT MISMATCH: {} — same aggSeq={}, hub_root={:#x}, verifier_root={:#x}",
                label, hub_agg_seq, hub_root, v_root
            );
            alerts.push(Alert {
                severity: Severity::Critical,
                domain: "crosschain".to_string(),
                title: format!("Root mismatch: {}", label),
                description: format!(
                    "Verifier **{}** (chain {}) has a **different root** at the same aggSeq {} as the hub.",
                    label, token.chain_id, v_agg_seq,
                ),
                fields: vec![
                    AlertField {
                        name: "Token".to_string(),
                        value: label.clone(),
                        inline: true,
                    },
                    AlertField {
                        name: "aggSeq".to_string(),
                        value: v_agg_seq.to_string(),
                        inline: true,
                    },
                    AlertField {
                        name: "Hub root".to_string(),
                        value: format!("{:#x}", hub_root),
                        inline: false,
                    },
                    AlertField {
                        name: "Verifier root".to_string(),
                        value: format!("{:#x}", v_root),
                        inline: false,
                    },
                ],
            });
            continue;
        }

        // Delay check: verifier is behind the hub
        if v_agg_seq < hub_agg_seq {
            let first_missing_seq = v_agg_seq + 1;

            // Look up the timestamp, using cache to avoid redundant RPC calls.
            let cached = ts_cache.get(&first_missing_seq).copied();
            let ts_result = match cached {
                Some(ts) => Ok(ts),
                None => hub
                    .aggregation_event_timestamp(
                        first_missing_seq,
                        HUB_EVENT_LOOKBACK_BLOCKS,
                        HUB_EVENT_CHUNK_SIZE,
                    )
                    .await
                    .map(|ts| {
                        ts_cache.insert(first_missing_seq, ts);
                        ts
                    }),
            };

            match ts_result {
                Ok(Some(event_ts)) => {
                    let delay = now.saturating_sub(event_ts);
                    if delay > root_delay_threshold {
                        let delay_min = delay / 60;
                        info!(
                            "ROOT SYNC DELAYED: {} — behind by {} seq(s), delay {}m (threshold {}s)",
                            label,
                            hub_agg_seq - v_agg_seq,
                            delay_min,
                            root_delay_threshold
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "crosschain".to_string(),
                            title: format!("Root sync delayed: {}", label),
                            description: format!(
                                "Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was emitted **{}m ago** (threshold {}m).",
                                label,
                                token.chain_id,
                                hub_agg_seq - v_agg_seq,
                                first_missing_seq,
                                delay_min,
                                root_delay_threshold / 60,
                            ),
                            fields: vec![
                                AlertField {
                                    name: "Token".to_string(),
                                    value: label.clone(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Chain".to_string(),
                                    value: token.chain_id.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Delay".to_string(),
                                    value: format!("{}m", delay_min),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Verifier aggSeq".to_string(),
                                    value: v_agg_seq.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Hub aggSeq".to_string(),
                                    value: hub_agg_seq.to_string(),
                                    inline: true,
                                },
                            ],
                        });
                    } else {
                        info!(
                            "Root propagating: {} — behind by {} seq(s), delay {}s (within threshold {}s)",
                            label,
                            hub_agg_seq - v_agg_seq,
                            delay,
                            root_delay_threshold,
                        );
                    }
                }
                Ok(None) => {
                    // Event not found in lookback window — very old lag
                    info!(
                        "ROOT SYNC DELAYED: {} — aggSeq {} not found in lookback (very old)",
                        label, first_missing_seq
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "crosschain".to_string(),
                        title: format!("Root sync delayed: {}", label),
                        description: format!(
                            "Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was not found in the last {} blocks (very old).",
                            label,
                            token.chain_id,
                            hub_agg_seq - v_agg_seq,
                            first_missing_seq,
                            HUB_EVENT_LOOKBACK_BLOCKS,
                        ),
                        fields: vec![
                            AlertField {
                                name: "Token".to_string(),
                                value: label.clone(),
                                inline: true,
                            },
                            AlertField {
                                name: "Chain".to_string(),
                                value: token.chain_id.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "Verifier aggSeq".to_string(),
                                value: v_agg_seq.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "Hub aggSeq".to_string(),
                                value: hub_agg_seq.to_string(),
                                inline: true,
                            },
                        ],
                    });
                }
                Err(err) => {
                    error!(
                        "failed to query hub event timestamp for '{}': {:?}",
                        label, err
                    );
                }
            }
        }
    }

    Ok(alerts)
}
