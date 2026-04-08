// SPDX-License-Identifier: BUSL-1.1

use anyhow::{Context, Result};
use candid::{Decode, Encode, Principal};
use ic_agent::Agent;
use ic_agent::identity::AnonymousIdentity;
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::IcpConfig;

pub async fn build_agent(replica_url: &str) -> Result<Agent> {
    let agent = Agent::builder()
        .with_identity(AnonymousIdentity)
        .with_url(replica_url)
        .build()
        .context("failed to build IC agent")?;
    Ok(agent)
}

async fn query_cycles(agent: &Agent, canister_id: &Principal) -> Result<u128> {
    let response = agent
        .query(canister_id, "get_cycles")
        .with_arg(Encode!().context("failed to encode empty args")?)
        .call()
        .await
        .context("get_cycles query failed")?;
    let cycles = Decode!(&response, u128).context("failed to decode get_cycles response")?;
    Ok(cycles)
}

fn format_cycles(cycles: u128) -> String {
    let trillion = 1_000_000_000_000u128;
    let whole = cycles / trillion;
    let frac = (cycles % trillion) / 1_000_000_000; // 3 decimal places
    format!("{}.{:03}T", whole, frac)
}

pub async fn check_icp_cycles(config: &IcpConfig) -> Vec<Alert> {
    let mut alerts = Vec::new();

    let agent = match build_agent(&config.replica_url).await {
        Ok(a) => a,
        Err(err) => {
            error!("failed to build IC agent: {:?}", err);
            return alerts;
        }
    };

    let threshold = config.cycle_threshold as u128;

    for canister in &config.canisters {
        let principal = match Principal::from_text(&canister.canister_id) {
            Ok(p) => p,
            Err(err) => {
                error!(
                    "invalid canister ID '{}' for '{}': {}",
                    canister.canister_id, canister.name, err
                );
                continue;
            }
        };

        match query_cycles(&agent, &principal).await {
            Ok(cycles) => {
                info!(
                    "canister {} ({}) cycles: {}",
                    canister.name,
                    canister.canister_id,
                    format_cycles(cycles)
                );

                if cycles < threshold / 2 {
                    alerts.push(build_alert(
                        Severity::Critical,
                        &canister.name,
                        &canister.canister_id,
                        cycles,
                        threshold,
                    ));
                } else if cycles < threshold {
                    alerts.push(build_alert(
                        Severity::Warning,
                        &canister.name,
                        &canister.canister_id,
                        cycles,
                        threshold,
                    ));
                }
            }
            Err(err) => {
                error!(
                    "failed to query cycles for '{}' ({}): {:?}",
                    canister.name, canister.canister_id, err
                );
            }
        }
    }

    alerts
}

fn build_alert(
    severity: Severity,
    name: &str,
    canister_id: &str,
    cycles: u128,
    threshold: u128,
) -> Alert {
    let cycles_str = format_cycles(cycles);
    let threshold_str = format_cycles(threshold);

    Alert {
        severity,
        domain: "icp_cycles".to_string(),
        title: format!("Low cycles: {}", name),
        description: format!(
            "Canister `{}` (`{}`) has **{}** cycles, below threshold of {}.",
            name, canister_id, cycles_str, threshold_str
        ),
        fields: vec![
            AlertField {
                name: "Canister".to_string(),
                value: name.to_string(),
                inline: true,
            },
            AlertField {
                name: "Canister ID".to_string(),
                value: canister_id.to_string(),
                inline: true,
            },
            AlertField {
                name: "Cycles".to_string(),
                value: cycles_str,
                inline: true,
            },
            AlertField {
                name: "Threshold".to_string(),
                value: threshold_str,
                inline: true,
            },
        ],
    }
}
