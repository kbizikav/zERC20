use std::collections::HashMap;
use std::str::FromStr;

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use anyhow::{Context, Result};
use client_common::contracts::utils::get_provider;
use log::{error, info};

use crate::alert::{Alert, AlertField, Severity};
use crate::config::{AccountConfig, ChainConfig};

/// Parse an ETH-denominated balance string (e.g., "0.01") into wei (U256).
fn parse_eth_to_wei(eth_str: &str) -> Result<U256> {
    let trimmed = eth_str.trim();

    let (integer_part, decimal_part) = match trimmed.split_once('.') {
        Some((i, d)) => (i, d),
        None => (trimmed, ""),
    };

    // Pad or truncate decimal to 18 digits
    let decimal_padded = if decimal_part.len() >= 18 {
        &decimal_part[..18]
    } else {
        &format!("{:0<18}", decimal_part)
    };

    let wei_str = format!("{}{}", integer_part, decimal_padded);
    let wei_str = wei_str.trim_start_matches('0');
    if wei_str.is_empty() {
        return Ok(U256::ZERO);
    }
    U256::from_str(wei_str).context(format!("failed to parse '{}' as ETH value", eth_str))
}

pub async fn check_balances(
    accounts: &[AccountConfig],
    chains: &HashMap<String, ChainConfig>,
) -> Vec<Alert> {
    let mut alerts = Vec::new();

    for account in accounts {
        let address = match Address::from_str(&account.address) {
            Ok(a) => a,
            Err(err) => {
                error!(
                    "invalid address '{}' for account '{}': {}",
                    account.address, account.name, err
                );
                continue;
            }
        };

        let threshold = match parse_eth_to_wei(&account.required_balance) {
            Ok(t) => t,
            Err(err) => {
                error!(
                    "invalid required_balance '{}' for account '{}': {}",
                    account.required_balance, account.name, err
                );
                continue;
            }
        };

        for chain_name in &account.chains {
            let chain_cfg = match chains.get(chain_name) {
                Some(c) => c,
                None => {
                    error!(
                        "chain '{}' not found in config for account '{}'",
                        chain_name, account.name
                    );
                    continue;
                }
            };

            match check_single_balance(&account.name, address, threshold, chain_name, chain_cfg)
                .await
            {
                Ok(Some(alert)) => alerts.push(alert),
                Ok(None) => {}
                Err(err) => {
                    error!(
                        "failed to check balance for '{}' on {}: {:?}",
                        account.name, chain_name, err
                    );
                }
            }
        }
    }

    alerts
}

async fn check_single_balance(
    account_name: &str,
    address: Address,
    threshold: U256,
    chain_name: &str,
    chain_cfg: &ChainConfig,
) -> Result<Option<Alert>> {
    let provider = get_provider(&chain_cfg.rpc_url)
        .with_context(|| format!("failed to create provider for chain '{}'", chain_name))?;

    let balance = provider
        .get_balance(address)
        .await
        .with_context(|| format!("failed to get balance on '{}'", chain_name))?;

    if balance < threshold {
        let balance_eth = format_wei_as_eth(balance);
        let threshold_eth = format_wei_as_eth(threshold);

        let explorer_link = chain_cfg
            .explorer
            .as_ref()
            .map(|base| format!("{}{}", base, address))
            .unwrap_or_default();

        info!(
            "LOW BALANCE: {} on {} = {} ETH (threshold: {} ETH)",
            account_name, chain_name, balance_eth, threshold_eth
        );

        let mut fields = vec![
            AlertField {
                name: "Account".to_string(),
                value: account_name.to_string(),
                inline: true,
            },
            AlertField {
                name: "Chain".to_string(),
                value: chain_name.to_string(),
                inline: true,
            },
            AlertField {
                name: "Balance".to_string(),
                value: format!("{} ETH", balance_eth),
                inline: true,
            },
            AlertField {
                name: "Threshold".to_string(),
                value: format!("{} ETH", threshold_eth),
                inline: true,
            },
        ];

        if !explorer_link.is_empty() {
            fields.push(AlertField {
                name: "Explorer".to_string(),
                value: explorer_link,
                inline: false,
            });
        }

        return Ok(Some(Alert {
            severity: Severity::Warning,
            domain: "balance".to_string(),
            title: format!("Low balance: {} on {}", account_name, chain_name),
            description: format!(
                "Account `{}` balance on **{}** is **{} ETH**, below threshold of {} ETH.",
                account_name, chain_name, balance_eth, threshold_eth
            ),
            fields,
        }));
    }

    Ok(None)
}

fn format_wei_as_eth(wei: U256) -> String {
    let one_eth = U256::from(10u64).pow(U256::from(18));
    let integer = wei / one_eth;
    let remainder = wei % one_eth;

    // Show 6 decimal places
    let scale = U256::from(10u64).pow(U256::from(12));
    let frac = remainder / scale;
    format!("{}.{:06}", integer, frac.to::<u64>())
}
