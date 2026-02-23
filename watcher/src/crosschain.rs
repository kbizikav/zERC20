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

        // aggSeq sync check
        match verifier.latest_agg_seq().await {
            Ok(v_agg_seq) => {
                if hub_agg_seq > v_agg_seq {
                    let delay = hub_agg_seq - v_agg_seq;
                    info!(
                        "AGG_SEQ DELAY: {} — hub={}, verifier={}, delay={}",
                        label, hub_agg_seq, v_agg_seq, delay
                    );
                    alerts.push(Alert {
                        severity: Severity::Warning,
                        domain: "crosschain".to_string(),
                        title: format!("aggSeq delay: {}", label),
                        description: format!(
                            "Verifier **{}** (chain {}) aggSeq is **{}** behind hub (hub={}, verifier={}).",
                            label, token.chain_id, delay, hub_agg_seq, v_agg_seq,
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
                                name: "Hub aggSeq".to_string(),
                                value: hub_agg_seq.to_string(),
                                inline: true,
                            },
                            AlertField {
                                name: "Verifier aggSeq".to_string(),
                                value: v_agg_seq.to_string(),
                                inline: true,
                            },
                        ],
                    });
                }

                // Root consistency check: only when verifier is at hub's seq
                if v_agg_seq == hub_agg_seq && hub_agg_seq > 0 {
                    match verifier.global_transfer_root(v_agg_seq).await {
                        Ok(v_root) => {
                            if v_root != hub_root && v_root != U256::ZERO {
                                info!(
                                    "ROOT MISMATCH: {} — hub_root={:#x}, verifier_root={:#x}",
                                    label, hub_root, v_root
                                );
                                alerts.push(Alert {
                                    severity: Severity::Critical,
                                    domain: "crosschain".to_string(),
                                    title: format!("Root mismatch: {}", label),
                                    description: format!(
                                        "Verifier **{}** (chain {}) root at aggSeq {} does not match hub root.",
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
                            }
                        }
                        Err(err) => {
                            error!(
                                "failed to read global_transfer_root for '{}': {:?}",
                                label, err
                            );
                        }
                    }
                }
            }
            Err(err) => {
                error!("failed to read latest_agg_seq for '{}': {:?}", label, err);
            }
        }

        // isUpToDate check
        match verifier.is_up_to_date().await {
            Ok(false) => {
                info!("NOT UP TO DATE: {}", label);
                alerts.push(Alert {
                    severity: Severity::Warning,
                    domain: "crosschain".to_string(),
                    title: format!("Not up to date: {}", label),
                    description: format!(
                        "Verifier **{}** (chain {}) reports `isUpToDate = false`.",
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
            }
            Ok(true) => {}
            Err(err) => {
                error!("failed to read is_up_to_date for '{}': {:?}", label, err);
            }
        }
    }

    Ok(alerts)
}
