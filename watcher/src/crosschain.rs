use anyhow::{Context, Result};
use client_common::{
    contracts::hub::{AggregationEventInfo, HubContract},
    contracts::verifier::VerifierContract,
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

/// Lookback window for verifier relay event queries.
/// L2 chains produce blocks much faster, so we use a larger window.
const VERIFIER_EVENT_LOOKBACK_BLOCKS: u64 = 500_000;

pub async fn check_crosschain(tokens: &[TokenConfig], root_delay_threshold: u64) -> Vec<Alert> {
    let mut alerts = Vec::new();

    for token in tokens {
        let path = match token.crosschain_config_path.as_ref() {
            Some(p) => p,
            None => continue,
        };
        match check_single_config(&token.name, path, root_delay_threshold).await {
            Ok(mut a) => alerts.append(&mut a),
            Err(err) => {
                error!(
                    "[{}] crosschain check failed for {}: {:?}",
                    token.name, path, err
                );
                alerts.push(Alert {
                    severity: Severity::Critical,
                    domain: "crosschain".to_string(),
                    title: format!("[{}] Config load failed: {}", token.name, path),
                    description: format!(
                        "[**{}**] Failed to load or process token config `{}`: {}",
                        token.name, path, err
                    ),
                    fields: vec![],
                });
            }
        }
    }

    alerts
}

async fn check_single_config(
    token_name: &str,
    path: &str,
    root_delay_threshold: u64,
) -> Result<Vec<Alert>> {
    let tokens_file: TokensFile =
        load_tokens_from_path(path).with_context(|| format!("loading {}", path))?;

    let hub_entry = tokens_file
        .hub
        .as_ref()
        .context("token config has no hub entry")?;

    let hub_provider = hub_entry.provider()?;
    let hub = HubContract::new(hub_provider, hub_entry.hub_address);

    let hub_agg_seq = hub.agg_seq().await.context("failed to read hub agg_seq")?;

    if hub_agg_seq == 0 {
        info!(
            "[{}] hub aggSeq is 0, skipping crosschain checks",
            token_name
        );
        return Ok(Vec::new());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut alerts = Vec::new();

    // Cache: aggSeq → Option<AggregationEventInfo>. Queried once per distinct
    // aggSeq, shared across all verifiers in this token config.
    let mut event_cache: HashMap<u64, Option<AggregationEventInfo>> = HashMap::new();

    for token in &tokens_file.tokens {
        let label = &token.label;
        let provider = match token.provider() {
            Ok(p) => p,
            Err(err) => {
                error!(
                    "[{}] failed to create provider for '{}': {:?}",
                    token_name, label, err
                );
                continue;
            }
        };

        let verifier = VerifierContract::new(provider, token.verifier_address);

        let v_agg_seq = match verifier.latest_agg_seq().await {
            Ok(seq) => seq,
            Err(err) => {
                error!(
                    "[{}] failed to read latest_agg_seq for '{}': {:?}",
                    token_name, label, err
                );
                continue;
            }
        };

        // ── Broadcast check (H->V) ──────────────────────────────────
        // Wrapped in a labeled block so that early exits (`break`) do
        // NOT skip the relay check that follows.
        'broadcast: {
            if v_agg_seq == 0 {
                info!(
                    "[{}] [H->V] No root synced: {} (aggSeq=0)",
                    token_name, label
                );
                alerts.push(Alert {
                    severity: Severity::Warning,
                    domain: "crosschain".to_string(),
                    title: format!("[{}] [H->V] Root not synced: {}", token_name, label),
                    description: format!(
                        "[**{}**] Verifier **{}** (chain {}) has not received any aggregation root yet (aggSeq=0).",
                        token_name, label, token.chain_id,
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
                break 'broadcast;
            }

            let v_root = match verifier.global_transfer_root(v_agg_seq).await {
                Ok(r) => r,
                Err(err) => {
                    error!(
                        "[{}] [H->V] failed to read global_transfer_root for '{}': {:?}",
                        token_name, label, err
                    );
                    break 'broadcast;
                }
            };

            // Look up the hub event for the verifier's aggSeq (for root comparison)
            // or the first missing aggSeq (for delay check). Uses cache to avoid
            // redundant RPC calls when multiple verifiers share the same aggSeq.
            let target_seq = if v_agg_seq == hub_agg_seq {
                v_agg_seq
            } else {
                v_agg_seq + 1
            };

            let cached = event_cache.get(&target_seq).copied();
            let event_result = match cached {
                Some(info) => Ok(info),
                None => hub
                    .aggregation_event_info(
                        target_seq,
                        HUB_EVENT_LOOKBACK_BLOCKS,
                        HUB_EVENT_CHUNK_SIZE,
                    )
                    .await
                    .map(|info| {
                        event_cache.insert(target_seq, info);
                        info
                    }),
            };

            if v_agg_seq == hub_agg_seq {
                // Critical: same aggSeq but different root → protocol-level inconsistency.
                // Compare against the root from the broadcast event, not currentAggregationRoot().
                match event_result {
                    Ok(Some(hub_event)) => {
                        if v_root != hub_event.root && v_root != alloy::primitives::U256::ZERO {
                            info!(
                                "[{}] [H->V] ROOT MISMATCH: {} — aggSeq={}, hub={:#x}, verifier={:#x}",
                                token_name, label, hub_agg_seq, hub_event.root, v_root
                            );
                            alerts.push(Alert {
                                severity: Severity::Critical,
                                domain: "crosschain".to_string(),
                                title: format!(
                                    "[{}] [H->V] Root mismatch: {}",
                                    token_name, label
                                ),
                                description: format!(
                                    "[**{}**] Verifier **{}** (chain {}) has a **different root** at the same aggSeq {} as the hub.",
                                    token_name, label, token.chain_id, v_agg_seq,
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
                                        value: format!("{:#x}", hub_event.root),
                                        inline: false,
                                    },
                                    AlertField {
                                        name: "Verifier root".to_string(),
                                        value: format!("{:#x}", v_root),
                                        inline: false,
                                    },
                                ],
                            });
                        }
                    }
                    Ok(None) => {
                        info!(
                            "[{}] [H->V] hub event for aggSeq {} not found in lookback for '{}'",
                            token_name, hub_agg_seq, label
                        );
                    }
                    Err(err) => {
                        error!(
                            "[{}] [H->V] failed to query hub event for '{}': {:?}",
                            token_name, label, err
                        );
                    }
                }
            } else {
                // Delay check: verifier is behind the hub (v_agg_seq < hub_agg_seq)
                let first_missing_seq = v_agg_seq + 1;

                match event_result {
                    Ok(Some(event_info)) => {
                        let delay = now.saturating_sub(event_info.block_timestamp);
                        if delay > root_delay_threshold {
                            let delay_min = delay / 60;
                            info!(
                                "[{}] [H->V] BROADCAST DELAYED: {} — behind {} seq(s), delay {}m (threshold {}m)",
                                token_name,
                                label,
                                hub_agg_seq - v_agg_seq,
                                delay_min,
                                root_delay_threshold / 60,
                            );
                            alerts.push(Alert {
                                severity: Severity::Warning,
                                domain: "crosschain".to_string(),
                                title: format!(
                                    "[{}] [H->V] Broadcast delayed: {}",
                                    token_name, label
                                ),
                                description: format!(
                                    "[**{}**] Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was emitted **{}m ago** (threshold {}m).",
                                    token_name,
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
                                "[{}] [H->V] Broadcast propagating: {} — behind {} seq(s), delay {}s (within {}s)",
                                token_name,
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
                            "[{}] [H->V] BROADCAST DELAYED: {} — aggSeq {} not found in lookback (very old)",
                            token_name, label, first_missing_seq
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "crosschain".to_string(),
                            title: format!(
                                "[{}] [H->V] Broadcast delayed: {}",
                                token_name, label
                            ),
                            description: format!(
                                "[**{}**] Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was not found in the last {} blocks (very old).",
                                token_name,
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
                            "[{}] [H->V] failed to query hub event for '{}': {:?}",
                            token_name, label, err
                        );
                    }
                }
            }
        }

        // ── Relay check (V->H) ──────────────────────────────────────
        let eid = match token.eid {
            Some(e) => e,
            None => continue,
        };

        let v_relayed_index = match verifier.latest_relayed_index().await {
            Ok(idx) => idx,
            Err(err) => {
                error!(
                    "[{}] [V->H] failed to read latest_relayed_index for '{}': {:?}",
                    token_name, label, err
                );
                continue;
            }
        };

        if v_relayed_index == 0 {
            continue;
        }

        let position = match hub.eid_position(eid).await {
            Ok(p) => p,
            Err(err) => {
                error!(
                    "[{}] [V->H] failed to read eid_position for '{}' (eid={}): {:?}",
                    token_name, label, eid, err
                );
                continue;
            }
        };

        if position == 0 {
            info!(
                "[{}] [V->H] eid_position is 0 for '{}' (eid={}), skipping relay check",
                token_name, label, eid
            );
            continue;
        }

        let hub_tree_index = match hub.transfer_tree_index(position - 1).await {
            Ok(idx) => idx,
            Err(err) => {
                error!(
                    "[{}] [V->H] failed to read transfer_tree_index for '{}': {:?}",
                    token_name, label, err
                );
                continue;
            }
        };

        if v_relayed_index == hub_tree_index {
            info!(
                "[{}] [V->H] Relay synced: {} — index={}",
                token_name, label, v_relayed_index
            );
        } else if v_relayed_index > hub_tree_index {
            let next_expected = hub_tree_index + 1;
            match verifier
                .relay_event_timestamp(
                    next_expected,
                    VERIFIER_EVENT_LOOKBACK_BLOCKS,
                    HUB_EVENT_CHUNK_SIZE,
                )
                .await
            {
                Ok(Some(event_ts)) => {
                    let delay = now.saturating_sub(event_ts);
                    if delay > root_delay_threshold {
                        let delay_min = delay / 60;
                        info!(
                            "[{}] [V->H] RELAY DELAYED: {} — relayed={}, hub={}, delay={}m",
                            token_name, label, v_relayed_index, hub_tree_index, delay_min
                        );
                        alerts.push(Alert {
                            severity: Severity::Warning,
                            domain: "crosschain".to_string(),
                            title: format!(
                                "[{}] [V->H] Relay delayed: {}",
                                token_name, label
                            ),
                            description: format!(
                                "[**{}**] Verifier **{}** (chain {}) relayed index {} but hub only received up to {}. Transfer root index {} was relayed **{}m ago** (threshold {}m).",
                                token_name,
                                label,
                                token.chain_id,
                                v_relayed_index,
                                hub_tree_index,
                                next_expected,
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
                                    name: "Verifier relayed".to_string(),
                                    value: v_relayed_index.to_string(),
                                    inline: true,
                                },
                                AlertField {
                                    name: "Hub received".to_string(),
                                    value: hub_tree_index.to_string(),
                                    inline: true,
                                },
                            ],
                        });
                    } else {
                        info!(
                            "[{}] [V->H] Relay propagating: {} — relayed={}, hub={}, delay={}s (within {}s)",
                            token_name,
                            label,
                            v_relayed_index,
                            hub_tree_index,
                            delay,
                            root_delay_threshold
                        );
                    }
                }
                Ok(None) => {
                    info!(
                        "[{}] [V->H] RELAY DELAYED: {} — index {} not found in lookback (very old)",
                        token_name, label, next_expected
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "crosschain".to_string(),
                        title: format!("[{}] [V->H] Relay delayed: {}", token_name, label),
                        description: format!(
                            "[**{}**] Verifier **{}** (chain {}) relayed index {} but hub only received up to {}. Relay event for index {} was not found in the last {} blocks (very old).",
                            token_name,
                            label,
                            token.chain_id,
                            v_relayed_index,
                            hub_tree_index,
                            next_expected,
                            VERIFIER_EVENT_LOOKBACK_BLOCKS,
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
                                name: "Verifier relayed".to_string(),
                                value: v_relayed_index.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "Hub received".to_string(),
                                value: hub_tree_index.to_string(),
                                inline: true,
                            },
                        ],
                    });
                }
                Err(err) => {
                    error!(
                        "[{}] [V->H] failed to query relay event for '{}': {:?}",
                        token_name, label, err
                    );
                }
            }
        }
    }

    Ok(alerts)
}
