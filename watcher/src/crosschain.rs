use anyhow::{Context, Result};
use client_common::{
    contracts::ContractResult,
    contracts::hub::{AggregationEventInfo, HubContract},
    contracts::verifier::{RelayEventInfo, VerifierContract},
    tokens::{TokensFile, load_tokens_from_path},
};
use log::{error, info};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::{CrosschainConfig, TokenConfig};

pub async fn check_crosschain(tokens: &[TokenConfig], cc: &CrosschainConfig) -> Vec<Alert> {
    let mut alerts = Vec::new();

    for token in tokens {
        let path = match token.crosschain_config_path.as_ref() {
            Some(p) => p,
            None => continue,
        };
        match check_single_config(&token.name, path, cc).await {
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
    cc: &CrosschainConfig,
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
        alerts.extend(
            check_broadcast(
                token_name,
                label,
                token.chain_id,
                &hub,
                &verifier,
                v_agg_seq,
                hub_agg_seq,
                now,
                cc,
                &mut event_cache,
            )
            .await,
        );

        // ── Relay check (V->H) ──────────────────────────────────────
        let eid = match token.eid {
            Some(e) => e,
            None => continue,
        };

        alerts.extend(
            check_relay(
                token_name,
                label,
                token.chain_id,
                &hub,
                &verifier,
                eid,
                now,
                cc,
            )
            .await,
        );
    }

    Ok(alerts)
}

// ─── Broadcast (H->V) ───────────────────────────────────────────────────────

/// Check whether the hub's aggregation root has been broadcast to this verifier.
async fn check_broadcast(
    token_name: &str,
    label: &str,
    chain_id: u64,
    hub: &HubContract,
    verifier: &VerifierContract,
    v_agg_seq: u64,
    hub_agg_seq: u64,
    now: u64,
    cc: &CrosschainConfig,
    event_cache: &mut HashMap<u64, Option<AggregationEventInfo>>,
) -> Vec<Alert> {
    if v_agg_seq == 0 {
        info!(
            "[{}] [H->V] No root synced: {} (aggSeq=0)",
            token_name, label
        );
        return vec![Alert {
            severity: Severity::Warning,
            domain: "crosschain".to_string(),
            title: format!("[{}] [H->V] Root not synced: {}", token_name, label),
            description: format!(
                "[**{}**] Verifier **{}** (chain {}) has not received any aggregation root yet (aggSeq=0).",
                token_name, label, chain_id,
            ),
            fields: vec![
                AlertField {
                    name: "Token".to_string(),
                    value: label.to_string(),
                    inline: true,
                },
                AlertField {
                    name: "Chain".to_string(),
                    value: chain_id.to_string(),
                    inline: true,
                },
            ],
        }];
    }

    let v_root = match verifier.global_transfer_root(v_agg_seq).await {
        Ok(r) => r,
        Err(err) => {
            error!(
                "[{}] [H->V] failed to read global_transfer_root for '{}': {:?}",
                token_name, label, err
            );
            return vec![];
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
                cc.hub_event_lookback_blocks,
                cc.hub_event_chunk_size,
            )
            .await
            .inspect(|&info| {
                event_cache.insert(target_seq, info);
            }),
    };

    // Same aggSeq → verify roots match.
    if v_agg_seq == hub_agg_seq {
        return check_broadcast_root(token_name, label, chain_id, v_agg_seq, v_root, event_result);
    }

    // Verifier is behind the hub → check delay.
    check_broadcast_delay(
        token_name,
        label,
        chain_id,
        hub,
        v_agg_seq,
        hub_agg_seq,
        now,
        cc,
        event_result,
    )
    .await
}

/// Critical: same aggSeq but different root → protocol-level inconsistency.
/// Compare against the root from the broadcast event, not currentAggregationRoot().
fn check_broadcast_root(
    token_name: &str,
    label: &str,
    chain_id: u64,
    v_agg_seq: u64,
    v_root: alloy::primitives::U256,
    event_result: ContractResult<Option<AggregationEventInfo>>,
) -> Vec<Alert> {
    match event_result {
        Ok(Some(hub_event))
            if v_root != hub_event.root && v_root != alloy::primitives::U256::ZERO =>
        {
            info!(
                "[{}] [H->V] ROOT MISMATCH: {} — aggSeq={}, hub={:#x}, verifier={:#x}",
                token_name, label, v_agg_seq, hub_event.root, v_root
            );
            vec![Alert {
                severity: Severity::Critical,
                domain: "crosschain".to_string(),
                title: format!("[{}] [H->V] Root mismatch: {}", token_name, label),
                description: format!(
                    "[**{}**] Verifier **{}** (chain {}) has a **different root** at the same aggSeq {} as the hub.",
                    token_name, label, chain_id, v_agg_seq,
                ),
                fields: vec![
                    AlertField {
                        name: "Token".to_string(),
                        value: label.to_string(),
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
            }]
        }
        Ok(None) => {
            info!(
                "[{}] [H->V] hub event for aggSeq {} not found in lookback for '{}'",
                token_name, v_agg_seq, label
            );
            vec![]
        }
        Err(err) => {
            error!(
                "[{}] [H->V] failed to query hub event for '{}': {:?}",
                token_name, label, err
            );
            vec![]
        }
        _ => vec![],
    }
}

/// Verifier is behind the hub (v_agg_seq < hub_agg_seq) — check how long
/// the first missing root has been available on the hub.
async fn check_broadcast_delay(
    token_name: &str,
    label: &str,
    chain_id: u64,
    hub: &HubContract,
    v_agg_seq: u64,
    hub_agg_seq: u64,
    now: u64,
    cc: &CrosschainConfig,
    event_result: ContractResult<Option<AggregationEventInfo>>,
) -> Vec<Alert> {
    let first_missing_seq = v_agg_seq + 1;
    let root_delay_threshold = cc.root_delay_threshold_seconds;

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
                vec![Alert {
                    severity: Severity::Warning,
                    domain: "crosschain".to_string(),
                    title: format!("[{}] [H->V] Broadcast delayed: {}", token_name, label),
                    description: format!(
                        "[**{}**] Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was emitted **{}m ago** (threshold {}m).",
                        token_name,
                        label,
                        chain_id,
                        hub_agg_seq - v_agg_seq,
                        first_missing_seq,
                        delay_min,
                        root_delay_threshold / 60,
                    ),
                    fields: vec![
                        AlertField {
                            name: "Token".to_string(),
                            value: label.to_string(),
                            inline: true,
                        },
                        AlertField {
                            name: "Chain".to_string(),
                            value: chain_id.to_string(),
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
                }]
            } else {
                info!(
                    "[{}] [H->V] Broadcast propagating: {} — behind {} seq(s), delay {}s (within {}s)",
                    token_name,
                    label,
                    hub_agg_seq - v_agg_seq,
                    delay,
                    root_delay_threshold,
                );
                vec![]
            }
        }
        Ok(None) => {
            // Event not found in lookback window — estimate minimum delay
            // from the timestamp of the earliest scanned block.
            let min_delay_desc =
                match hub_lookback_min_delay(hub, cc.hub_event_lookback_blocks, now).await {
                    Some(d) => format!("at least **{}m ago**", d / 60),
                    None => format!(
                        "not found in the last {} blocks",
                        cc.hub_event_lookback_blocks
                    ),
                };
            info!(
                "[{}] [H->V] BROADCAST DELAYED: {} — aggSeq {} {} (threshold {}m)",
                token_name,
                label,
                first_missing_seq,
                min_delay_desc,
                root_delay_threshold / 60,
            );
            vec![Alert {
                severity: Severity::Warning,
                domain: "crosschain".to_string(),
                title: format!("[{}] [H->V] Broadcast delayed: {}", token_name, label),
                description: format!(
                    "[**{}**] Verifier **{}** (chain {}) is behind by {} seq(s). First missing aggSeq {} was emitted {} (threshold {}m).",
                    token_name,
                    label,
                    chain_id,
                    hub_agg_seq - v_agg_seq,
                    first_missing_seq,
                    min_delay_desc,
                    root_delay_threshold / 60,
                ),
                fields: vec![
                    AlertField {
                        name: "Token".to_string(),
                        value: label.to_string(),
                        inline: true,
                    },
                    AlertField {
                        name: "Chain".to_string(),
                        value: chain_id.to_string(),
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
            }]
        }
        Err(err) => {
            error!(
                "[{}] [H->V] failed to query hub event for '{}': {:?}",
                token_name, label, err
            );
            vec![]
        }
    }
}

// ─── Relay (V->H) ───────────────────────────────────────────────────────────

/// Check whether the verifier's relayed transfer roots have been delivered to
/// the hub.
async fn check_relay(
    token_name: &str,
    label: &str,
    chain_id: u64,
    hub: &HubContract,
    verifier: &VerifierContract,
    eid: u32,
    now: u64,
    cc: &CrosschainConfig,
) -> Vec<Alert> {
    let v_relayed_index = match verifier.latest_relayed_index().await {
        Ok(idx) => idx,
        Err(err) => {
            error!(
                "[{}] [V->H] failed to read latest_relayed_index for '{}': {:?}",
                token_name, label, err
            );
            return vec![];
        }
    };

    if v_relayed_index == 0 {
        return vec![];
    }

    let position = match hub.eid_position(eid).await {
        Ok(p) => p,
        Err(err) => {
            error!(
                "[{}] [V->H] failed to read eid_position for '{}' (eid={}): {:?}",
                token_name, label, eid, err
            );
            return vec![];
        }
    };

    if position == 0 {
        info!(
            "[{}] [V->H] eid_position is 0 for '{}' (eid={}), skipping relay check",
            token_name, label, eid
        );
        return vec![];
    }

    let hub_tree_index = match hub.transfer_tree_index(position - 1).await {
        Ok(idx) => idx,
        Err(err) => {
            error!(
                "[{}] [V->H] failed to read transfer_tree_index for '{}': {:?}",
                token_name, label, err
            );
            return vec![];
        }
    };

    if v_relayed_index == hub_tree_index {
        // Index is synced — verify that the roots match.
        return match check_relay_root_match(
            token_name,
            label,
            chain_id,
            verifier,
            hub,
            v_relayed_index,
            position,
        )
        .await
        {
            Some(alert) => vec![alert],
            None => {
                info!(
                    "[{}] [V->H] Relay synced: {} — index={}",
                    token_name, label, v_relayed_index
                );
                vec![]
            }
        };
    }

    if v_relayed_index < hub_tree_index {
        info!(
            "[{}] [V->H] INDEX INCONSISTENCY: {} — v_relayed={}, hub={}",
            token_name, label, v_relayed_index, hub_tree_index
        );
        return vec![Alert {
            severity: Severity::Critical,
            domain: "crosschain".to_string(),
            title: format!(
                "[{}] [V->H] Relay index inconsistency: {}",
                token_name, label
            ),
            description: format!(
                "[**{}**] Verifier **{}** (chain {}) has relayed index {} but hub received up to {}. Hub has more than verifier relayed.",
                token_name, label, chain_id, v_relayed_index, hub_tree_index,
            ),
            fields: vec![
                AlertField {
                    name: "Token".to_string(),
                    value: label.to_string(),
                    inline: true,
                },
                AlertField {
                    name: "Chain".to_string(),
                    value: chain_id.to_string(),
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
        }];
    }

    // v_relayed_index > hub_tree_index — check relay delay.
    check_relay_delay(
        token_name,
        label,
        chain_id,
        verifier,
        v_relayed_index,
        hub_tree_index,
        now,
        cc,
    )
    .await
}

/// Verifier has relayed more than the hub has received — find the oldest
/// pending relay event (index > hub_tree_index) and check its age.
async fn check_relay_delay(
    token_name: &str,
    label: &str,
    chain_id: u64,
    verifier: &VerifierContract,
    v_relayed_index: u64,
    hub_tree_index: u64,
    now: u64,
    cc: &CrosschainConfig,
) -> Vec<Alert> {
    let root_delay_threshold = cc.root_delay_threshold_seconds;

    match verifier
        .oldest_pending_relay_event(
            hub_tree_index,
            cc.verifier_event_lookback_blocks,
            cc.verifier_event_chunk_size,
        )
        .await
    {
        Ok(Some(RelayEventInfo {
            index: oldest_index,
            block_timestamp,
        })) => {
            let delay = now.saturating_sub(block_timestamp);
            if delay > root_delay_threshold {
                let delay_min = delay / 60;
                info!(
                    "[{}] [V->H] RELAY DELAYED: {} — relayed={}, hub={}, oldest_pending={}, delay={}m",
                    token_name, label, v_relayed_index, hub_tree_index, oldest_index, delay_min
                );
                vec![Alert {
                    severity: Severity::Warning,
                    domain: "crosschain".to_string(),
                    title: format!("[{}] [V->H] Relay delayed: {}", token_name, label),
                    description: format!(
                        "[**{}**] Verifier **{}** (chain {}) relayed index {} but hub only received up to {}. Oldest pending relay (index {}) was emitted **{}m ago** (threshold {}m).",
                        token_name,
                        label,
                        chain_id,
                        v_relayed_index,
                        hub_tree_index,
                        oldest_index,
                        delay_min,
                        root_delay_threshold / 60,
                    ),
                    fields: vec![
                        AlertField {
                            name: "Token".to_string(),
                            value: label.to_string(),
                            inline: true,
                        },
                        AlertField {
                            name: "Chain".to_string(),
                            value: chain_id.to_string(),
                            inline: true,
                        },
                        AlertField {
                            name: "Delay".to_string(),
                            value: format!("{}m", delay_min),
                            inline: true,
                        },
                        AlertField {
                            name: "Oldest pending".to_string(),
                            value: oldest_index.to_string(),
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
                }]
            } else {
                info!(
                    "[{}] [V->H] Relay propagating: {} — relayed={}, hub={}, delay={}s (within {}s)",
                    token_name, label, v_relayed_index, hub_tree_index, delay, root_delay_threshold
                );
                vec![]
            }
        }
        Ok(None) => {
            // No pending relay event found in lookback window — estimate
            // minimum delay from the earliest scanned block.
            let min_delay_desc =
                match verifier_lookback_min_delay(verifier, cc.verifier_event_lookback_blocks, now)
                    .await
                {
                    Some(d) => format!("at least **{}m ago**", d / 60),
                    None => format!(
                        "not found in the last {} blocks",
                        cc.verifier_event_lookback_blocks
                    ),
                };
            info!(
                "[{}] [V->H] RELAY DELAYED: {} — no pending relay event in lookback, {} (threshold {}m)",
                token_name,
                label,
                min_delay_desc,
                root_delay_threshold / 60,
            );
            vec![Alert {
                severity: Severity::Warning,
                domain: "crosschain".to_string(),
                title: format!("[{}] [V->H] Relay delayed: {}", token_name, label),
                description: format!(
                    "[**{}**] Verifier **{}** (chain {}) relayed index {} but hub only received up to {}. Oldest pending relay {} (threshold {}m).",
                    token_name,
                    label,
                    chain_id,
                    v_relayed_index,
                    hub_tree_index,
                    min_delay_desc,
                    root_delay_threshold / 60,
                ),
                fields: vec![
                    AlertField {
                        name: "Token".to_string(),
                        value: label.to_string(),
                        inline: true,
                    },
                    AlertField {
                        name: "Chain".to_string(),
                        value: chain_id.to_string(),
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
            }]
        }
        Err(err) => {
            error!(
                "[{}] [V->H] failed to query relay event for '{}': {:?}",
                token_name, label, err
            );
            vec![]
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Compare the transfer root on the verifier (proved) against the root stored
/// on the hub for the same index. A mismatch is a critical protocol-level
/// inconsistency.
async fn check_relay_root_match(
    token_name: &str,
    label: &str,
    chain_id: u64,
    verifier: &VerifierContract,
    hub: &HubContract,
    relayed_index: u64,
    position: u64,
) -> Option<Alert> {
    let v_root = match verifier.proved_transfer_root(relayed_index).await {
        Ok(r) => r,
        Err(err) => {
            error!(
                "[{}] [V->H] failed to read proved_transfer_root for '{}': {:?}",
                token_name, label, err
            );
            return None;
        }
    };

    let hub_root = match hub.transfer_root(position - 1).await {
        Ok(r) => r,
        Err(err) => {
            error!(
                "[{}] [V->H] failed to read hub transfer_root for '{}': {:?}",
                token_name, label, err
            );
            return None;
        }
    };

    if v_root == hub_root {
        return None;
    }

    info!(
        "[{}] [V->H] ROOT MISMATCH: {} — index={}, verifier={:#x}, hub={:#x}",
        token_name, label, relayed_index, v_root, hub_root
    );

    Some(Alert {
        severity: Severity::Critical,
        domain: "crosschain".to_string(),
        title: format!("[{}] [V->H] Relay root mismatch: {}", token_name, label),
        description: format!(
            "[**{}**] Verifier **{}** (chain {}) proved root differs from hub root at the same index {}.",
            token_name, label, chain_id, relayed_index,
        ),
        fields: vec![
            AlertField {
                name: "Token".to_string(),
                value: label.to_string(),
                inline: true,
            },
            AlertField {
                name: "Index".to_string(),
                value: relayed_index.to_string(),
                inline: true,
            },
            AlertField {
                name: "Verifier root".to_string(),
                value: format!("{:#x}", v_root),
                inline: false,
            },
            AlertField {
                name: "Hub root".to_string(),
                value: format!("{:#x}", hub_root),
                inline: false,
            },
        ],
    })
}

/// Estimate the minimum delay for a hub event that fell outside the lookback
/// window by fetching the timestamp of the earliest scanned block.
/// Returns `Some(seconds)` or `None` if the RPC call fails.
async fn hub_lookback_min_delay(hub: &HubContract, lookback_blocks: u64, now: u64) -> Option<u64> {
    let latest = hub.latest_block().await.ok()?;
    let earliest = latest.saturating_sub(lookback_blocks);
    let ts = hub.block_timestamp(earliest).await.ok()?;
    Some(now.saturating_sub(ts))
}

/// Estimate the minimum delay for a verifier relay event that fell outside
/// the lookback window.
async fn verifier_lookback_min_delay(
    verifier: &VerifierContract,
    lookback_blocks: u64,
    now: u64,
) -> Option<u64> {
    let latest = verifier.latest_block().await.ok()?;
    let earliest = latest.saturating_sub(lookback_blocks);
    let ts = verifier.block_timestamp(earliest).await.ok()?;
    Some(now.saturating_sub(ts))
}
