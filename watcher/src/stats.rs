use std::collections::HashMap;
use std::str::FromStr;

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use anyhow::{Context, Result};
use api_types::indexer::TokenStatusResponse;
use client_common::{
    contracts::{hub::HubContract, utils::get_provider, verifier::VerifierContract},
    tokens::{TokensFile, load_tokens_from_path},
};
use log::{error, info};

use crate::alert::{DiscordEmbed, DiscordField};
use crate::balance::format_wei_as_eth;
use crate::config::{AccountConfig, ChainConfig, TokenConfig, WatcherConfig};

/// Collect system-wide stats and return Discord embeds for reporting.
pub async fn collect_stats(config: &WatcherConfig) -> Vec<DiscordEmbed> {
    let mut embeds = Vec::new();

    if !config.accounts.is_empty() {
        info!("collecting balance stats...");
        match collect_balance_stats(&config.accounts, &config.chains).await {
            Some(embed) => embeds.push(embed),
            None => {}
        }
    }

    if config.indexer.is_some() && !config.tokens.is_empty() {
        info!("collecting indexer stats...");
        if let Some(embed) = collect_indexer_stats(&config.tokens).await {
            embeds.push(embed);
        }
    }

    let has_crosschain = config
        .tokens
        .iter()
        .any(|t| t.crosschain_config_path.is_some());
    if has_crosschain {
        info!("collecting crosschain stats...");
        if let Some(embed) = collect_crosschain_stats(&config.tokens).await {
            embeds.push(embed);
        }
    }

    embeds
}

/// Balance stats: current balance for each account × chain.
async fn collect_balance_stats(
    accounts: &[AccountConfig],
    chains: &HashMap<String, ChainConfig>,
) -> Option<DiscordEmbed> {
    let mut lines = Vec::new();

    for account in accounts {
        let address = match Address::from_str(&account.address) {
            Ok(a) => a,
            Err(err) => {
                error!("invalid address '{}': {}", account.address, err);
                continue;
            }
        };

        for chain_name in &account.chains {
            let chain_cfg = match chains.get(chain_name) {
                Some(c) => c,
                None => continue,
            };

            match get_balance(address, chain_cfg).await {
                Ok(balance) => {
                    let eth = format_wei_as_eth(balance);
                    lines.push(format!(
                        "{:<20} {:>10} ETH  ({})",
                        account.name, eth, chain_name
                    ));
                }
                Err(err) => {
                    error!(
                        "failed to get balance for '{}' on {}: {:?}",
                        account.name, chain_name, err
                    );
                    lines.push(format!(
                        "{:<20} {:>10}      ({})",
                        account.name, "ERROR", chain_name
                    ));
                }
            }
        }
    }

    if lines.is_empty() {
        return None;
    }

    Some(DiscordEmbed {
        title: "Balance Stats".to_string(),
        description: format!("```\n{}\n```", lines.join("\n")),
        color: 0x2ECC71,
        fields: vec![],
    })
}

async fn get_balance(address: Address, chain_cfg: &ChainConfig) -> Result<U256> {
    let provider = get_provider(&chain_cfg.rpc_url).context("failed to create provider")?;
    provider
        .get_balance(address)
        .await
        .context("failed to get balance")
}

/// Indexer stats: pipeline stage indices for each token.
async fn collect_indexer_stats(tokens: &[TokenConfig]) -> Option<DiscordEmbed> {
    let client = reqwest::Client::new();
    let mut token_statuses: Vec<(String, Vec<TokenStatusResponse>)> = Vec::new();

    for token in tokens {
        let url = match token.indexer_url.as_ref() {
            Some(u) => u,
            None => continue,
        };
        match client
            .get(url)
            .send()
            .await
            .and_then(|r| Ok(r.error_for_status()?))
        {
            Ok(resp) => match resp.json::<Vec<TokenStatusResponse>>().await {
                Ok(s) => token_statuses.push((token.name.clone(), s)),
                Err(err) => {
                    error!(
                        "failed to parse indexer status for {}: {:?}",
                        token.name, err
                    );
                }
            },
            Err(err) => {
                error!(
                    "failed to fetch indexer status for {}: {:?}",
                    token.name, err
                );
            }
        }
    }

    if token_statuses.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "token", "events", "tree", "ivc", "reserve", "proved"
    ));
    lines.push("-".repeat(60));

    let fmt = |v: Option<u64>| match v {
        Some(n) => n.to_string(),
        None => "-".to_string(),
    };

    for (name, statuses) in &token_statuses {
        for s in statuses {
            lines.push(format!(
                "{:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
                name,
                fmt(s.events_synced_index),
                fmt(s.tree_synced_index),
                fmt(s.ivc_generated_index),
                fmt(s.onchain_reserved_index),
                fmt(s.onchain_proved_index),
            ));
        }
    }

    Some(DiscordEmbed {
        title: "Indexer Stats".to_string(),
        description: format!("```\n{}\n```", lines.join("\n")),
        color: 0x3498DB,
        fields: vec![],
    })
}

/// Crosschain stats: hub aggSeq/root and verifier sync status.
async fn collect_crosschain_stats(tokens: &[TokenConfig]) -> Option<DiscordEmbed> {
    let mut fields = Vec::new();

    for token in tokens {
        let path = match token.crosschain_config_path.as_ref() {
            Some(p) => p,
            None => continue,
        };
        match collect_single_crosschain(&token.name, path).await {
            Ok(Some(field)) => fields.push(field),
            Ok(None) => {}
            Err(err) => {
                error!("crosschain stats failed for {}: {:?}", token.name, err);
                fields.push(DiscordField {
                    name: token.name.clone(),
                    value: "```\nERROR\n```".to_string(),
                    inline: false,
                });
            }
        }
    }

    if fields.is_empty() {
        return None;
    }

    Some(DiscordEmbed {
        title: "Crosschain Stats".to_string(),
        description: String::new(),
        color: 0x3498DB,
        fields,
    })
}

async fn collect_single_crosschain(name: &str, path: &str) -> Result<Option<DiscordField>> {
    let tokens_file: TokensFile =
        load_tokens_from_path(path).with_context(|| format!("loading {}", path))?;

    let hub_entry = match tokens_file.hub.as_ref() {
        Some(h) => h,
        None => return Ok(None),
    };

    let hub_provider = hub_entry.provider()?;
    let hub = HubContract::new(hub_provider, hub_entry.hub_address);
    let hub_agg_seq = hub.agg_seq().await.context("hub agg_seq")?;
    let hub_root = hub
        .current_aggregation_root()
        .await
        .context("hub current_aggregation_root")?;

    let mut lines = Vec::new();
    lines.push(format!("Hub  aggSeq={} root={:#x}", hub_agg_seq, hub_root));

    for token in &tokens_file.tokens {
        let label = &token.label;
        let provider = match token.provider() {
            Ok(p) => p,
            Err(err) => {
                lines.push(format!("{}: ERROR ({})", label, err));
                continue;
            }
        };

        let verifier = VerifierContract::new(provider, token.verifier_address);
        let v_agg_seq = match verifier.latest_agg_seq().await {
            Ok(s) => s,
            Err(err) => {
                lines.push(format!("{}: ERROR ({})", label, err));
                continue;
            }
        };

        let root_status = if v_agg_seq == 0 {
            "no root".to_string()
        } else {
            match verifier.global_transfer_root(v_agg_seq).await {
                Ok(v_root) if v_root == hub_root && v_root != U256::ZERO => "synced".to_string(),
                Ok(_) => "not synced".to_string(),
                Err(_) => "error".to_string(),
            }
        };

        lines.push(format!(
            "{}: aggSeq={} root={}",
            label, v_agg_seq, root_status
        ));
    }

    Ok(Some(DiscordField {
        name: name.to_string(),
        value: format!("```\n{}\n```", lines.join("\n")),
        inline: false,
    }))
}
