use alloy::primitives::U256;
use anyhow::{Context, Result};
use client_common::{
    contracts::{hub::HubContract, verifier::VerifierContract},
    tokens::{TokensFile, load_tokens_from_path},
};
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::CrosschainConfig;

pub async fn check_crosschain(config: &CrosschainConfig) -> Vec<Alert> {
    let mut alerts = Vec::new();

    for path in &config.token_config_paths {
        match check_single_config(path).await {
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

async fn check_single_config(path: &str) -> Result<Vec<Alert>> {
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

    let mut alerts = Vec::new();

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

        // Root sync check: uses the same logic as crosschain-job's has_current_root().
        // A verifier is in sync if its root at latest_agg_seq matches the hub's current root,
        // regardless of whether the aggSeq values match.
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

        // Warning: verifier's latest root doesn't match hub's current root.
        // This means the broadcast hasn't reached this verifier yet.
        let has_current_root = v_root == hub_root && v_root != U256::ZERO;
        if !has_current_root {
            info!(
                "ROOT NOT SYNCED: {} — verifier_root={:#x} (seq={}), hub_root={:#x} (seq={})",
                label, v_root, v_agg_seq, hub_root, hub_agg_seq
            );
            alerts.push(Alert {
                severity: Severity::Warning,
                domain: "crosschain".to_string(),
                title: format!("Root not synced: {}", label),
                description: format!(
                    "Verifier **{}** (chain {}) does not have the hub's current aggregation root. Broadcast may be pending.",
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
        }
    }

    Ok(alerts)
}
