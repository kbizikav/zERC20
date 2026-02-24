use std::collections::HashMap;
use std::str::FromStr;

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use anyhow::{Context, Result};
use api_types::indexer::TokenStatusResponse;
use client_common::{
    contracts::{
        hub::HubContract, utils::get_provider, verifier::VerifierContract, z_erc20::ZErc20Contract,
    },
    tokens::{TokenEntry, TokensFile, load_tokens_from_path},
};
use log::{error, info};

use candid::{Decode, Encode, Principal};

use crate::alert::{DiscordEmbed, DiscordField};
use crate::balance::format_wei_as_eth;
use crate::config::{AccountConfig, ChainConfig, IcpConfig, TokenConfig, WatcherConfig};
use crate::icp_cycles;

/// Collect system-wide stats and return Discord embeds for reporting.
pub async fn collect_stats(config: &WatcherConfig) -> Vec<DiscordEmbed> {
    let mut embeds = Vec::new();

    if !config.accounts.is_empty() {
        info!("collecting balance stats...");
        if let Some(embed) = collect_balance_stats(&config.accounts, &config.chains).await {
            embeds.push(embed);
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

    if let Some(icp_config) = &config.icp {
        info!("collecting ICP cycle stats...");
        if let Some(embed) = collect_icp_stats(icp_config).await {
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

/// Indexer stats: on-chain index, tree_synced_index, and latestProvedIndex per chain.
async fn collect_indexer_stats(tokens: &[TokenConfig]) -> Option<DiscordEmbed> {
    let client = reqwest::Client::new();

    let fmt = |v: Option<u64>| match v {
        Some(n) => n.to_string(),
        None => "-".to_string(),
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<8} {:<12} {:>8} {:>8} {:>8}",
        "token", "chain", "onchain", "synced", "proved"
    ));
    lines.push("-".repeat(48));

    let mut has_data = false;

    for token in tokens {
        let base_url = match token.indexer_url.as_ref() {
            Some(u) => u.trim_end_matches('/'),
            None => continue,
        };

        // Fetch per-chain statuses from indexer
        let status_url = format!("{}/status", base_url);
        let statuses: Vec<TokenStatusResponse> = match client
            .get(&status_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => match resp.json().await {
                Ok(s) => s,
                Err(err) => {
                    error!(
                        "failed to parse indexer status for {}: {:?}",
                        token.name, err
                    );
                    continue;
                }
            },
            Err(err) => {
                error!(
                    "failed to fetch indexer status for {}: {:?}",
                    token.name, err
                );
                continue;
            }
        };

        // Load crosschain config for contract access
        let token_entries: Vec<TokenEntry> = match &token.crosschain_config_path {
            Some(path) => match load_tokens_from_path(path) {
                Ok(tf) => tf.tokens,
                Err(err) => {
                    error!(
                        "failed to load crosschain config for stats {}: {:?}",
                        token.name, err
                    );
                    vec![]
                }
            },
            None => vec![],
        };

        for status in &statuses {
            let entry = token_entries.iter().find(|e| e.chain_id == status.chain_id);
            let chain_label = entry.map(|e| e.label.as_str()).unwrap_or("?");

            let onchain = match entry {
                Some(e) => fetch_onchain_index(e).await.ok(),
                None => None,
            };

            let proved = match entry {
                Some(e) => fetch_proved_index(e).await.ok(),
                None => None,
            };

            lines.push(format!(
                "{:<8} {:<12} {:>8} {:>8} {:>8}",
                token.name,
                chain_label,
                fmt(onchain),
                fmt(status.tree_synced_index),
                fmt(proved),
            ));
            has_data = true;
        }
    }

    if !has_data {
        return None;
    }

    Some(DiscordEmbed {
        title: "Indexer Stats".to_string(),
        description: format!("```\n{}\n```", lines.join("\n")),
        color: 0x3498DB,
        fields: vec![],
    })
}

/// Fetch on-chain tree index from the zERC20 token contract.
async fn fetch_onchain_index(entry: &TokenEntry) -> Result<u64> {
    let provider = entry.provider()?;
    let contract = ZErc20Contract::new(provider, entry.token_address);
    contract.index().await.context("ZErc20.index() call failed")
}

/// Fetch latestProvedIndex from the verifier contract.
async fn fetch_proved_index(entry: &TokenEntry) -> Result<u64> {
    let provider = entry.provider()?;
    let verifier = VerifierContract::new(provider, entry.verifier_address);
    verifier
        .latest_proved_index()
        .await
        .context("Verifier.latestProvedIndex() call failed")
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

/// Crosschain stats formatted as a pipeline table:
///
/// ```text
/// Hub aggSeq=5
///
///              V->H relay   H->V broadcast
/// base          100->100          5->5
/// arb           100->95 (-5)      5->4 (-1)
/// scroll             -           5->3 (-2)
/// ```
///
/// V->H relay  : relayed -> hub_received  (transfer root delivery)
/// H->V broadcast: hub_aggSeq -> v_aggSeq (aggregation root delivery)
/// Gap counts (-N) are shown when the destination is behind.
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

    // Collect per-chain rows: (label, relay_col, broadcast_col)
    let mut rows: Vec<(String, String, String)> = Vec::new();

    for token in &tokens_file.tokens {
        let label = token.label.clone();
        let provider = match token.provider() {
            Ok(p) => p,
            Err(_) => {
                rows.push((label, "err".into(), "err".into()));
                continue;
            }
        };

        let verifier = VerifierContract::new(provider, token.verifier_address);

        // H->V broadcast: hub_aggSeq -> v_aggSeq
        let broadcast_col = match verifier.latest_agg_seq().await {
            Ok(v_seq) if v_seq == hub_agg_seq => format!("{}->{}", hub_agg_seq, v_seq),
            Ok(v_seq) => format!(
                "{}->{} (-{})",
                hub_agg_seq,
                v_seq,
                hub_agg_seq.saturating_sub(v_seq)
            ),
            Err(_) => "err".into(),
        };

        // V->H relay: v_relayed -> hub_received
        let relay_col = match token.eid {
            None => "-".into(),
            Some(eid) => match verifier.latest_relayed_index().await {
                Ok(0) => "-".into(),
                Ok(v_relayed) => match hub.eid_position(eid).await {
                    Ok(pos) if pos > 0 => match hub.transfer_tree_index(pos - 1).await {
                        Ok(hub_idx) if hub_idx == v_relayed => {
                            format!("{}->{}", v_relayed, hub_idx)
                        }
                        Ok(hub_idx) => format!(
                            "{}->{} (-{})",
                            v_relayed,
                            hub_idx,
                            v_relayed.saturating_sub(hub_idx)
                        ),
                        Err(_) => "err".into(),
                    },
                    _ => "err".into(),
                },
                Err(_) => "err".into(),
            },
        };

        rows.push((label, relay_col, broadcast_col));
    }

    if rows.is_empty() {
        return Ok(None);
    }

    // Calculate column widths for alignment
    let lw = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(5);
    let rw = rows
        .iter()
        .map(|r| r.1.len())
        .max()
        .unwrap_or(0)
        .max("V->H relay".len());
    let bw = rows
        .iter()
        .map(|r| r.2.len())
        .max()
        .unwrap_or(0)
        .max("H->V broadcast".len());

    let mut lines = Vec::new();
    lines.push(format!("Hub aggSeq={}", hub_agg_seq));
    lines.push(String::new());
    lines.push(format!(
        "{0:<1$}  {2:>3$}  {4:>5$}",
        "", lw, "V->H relay", rw, "H->V broadcast", bw,
    ));
    for (label, relay, broadcast) in &rows {
        lines.push(format!(
            "{0:<1$}  {2:>3$}  {4:>5$}",
            label, lw, relay, rw, broadcast, bw,
        ));
    }

    Ok(Some(DiscordField {
        name: name.to_string(),
        value: format!("```\n{}\n```", lines.join("\n")),
        inline: false,
    }))
}

/// ICP canister cycle stats.
async fn collect_icp_stats(config: &IcpConfig) -> Option<DiscordEmbed> {
    let agent = match icp_cycles::build_agent(&config.replica_url).await {
        Ok(a) => a,
        Err(err) => {
            error!("failed to build IC agent for stats: {:?}", err);
            return None;
        }
    };

    let mut lines = Vec::new();

    for canister in &config.canisters {
        let principal = match Principal::from_text(&canister.canister_id) {
            Ok(p) => p,
            Err(err) => {
                error!("invalid canister ID '{}': {}", canister.canister_id, err);
                lines.push(format!("{:<30} ERROR", canister.name));
                continue;
            }
        };

        match query_canister_cycles(&agent, &principal).await {
            Ok(cycles) => {
                let formatted = format_cycles_for_stats(cycles);
                lines.push(format!("{:<30} {}", canister.name, formatted));
            }
            Err(err) => {
                error!(
                    "failed to query cycles for '{}': {:?}",
                    canister.name, err
                );
                lines.push(format!("{:<30} ERROR", canister.name));
            }
        }
    }

    if lines.is_empty() {
        return None;
    }

    Some(DiscordEmbed {
        title: "ICP Canister Cycles".to_string(),
        description: format!("```\n{}\n```", lines.join("\n")),
        color: 0x9B59B6,
        fields: vec![],
    })
}

async fn query_canister_cycles(agent: &ic_agent::Agent, canister_id: &Principal) -> Result<u128> {
    let response = agent
        .query(canister_id, "get_cycles")
        .with_arg(Encode!().context("failed to encode args")?)
        .call()
        .await
        .context("get_cycles query failed")?;
    Decode!(&response, u128).context("failed to decode get_cycles response")
}

fn format_cycles_for_stats(cycles: u128) -> String {
    let trillion = 1_000_000_000_000u128;
    let whole = cycles / trillion;
    let frac = (cycles % trillion) / 1_000_000_000;
    format!("{}.{:03}T cycles", whole, frac)
}
