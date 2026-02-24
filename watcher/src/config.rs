use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use client_common::tokens::expand_env_vars;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WatcherConfig {
    pub discord_webhook_url: String,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub stats_interval_seconds: Option<u64>,

    #[serde(default)]
    pub alert: AlertConfig,

    #[serde(default)]
    pub chains: HashMap<String, ChainConfig>,
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,

    #[serde(default)]
    pub indexer: Option<IndexerConfig>,

    #[serde(default)]
    pub crosschain: Option<CrosschainConfig>,

    #[serde(default)]
    pub icp: Option<IcpConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenConfig {
    pub name: String,
    #[serde(default)]
    pub indexer_url: Option<String>,
    #[serde(default)]
    pub crosschain_config_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccountConfig {
    pub name: String,
    pub address: String,
    pub required_balance: String,
    pub chains: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChainConfig {
    pub rpc_url: String,
    #[serde(default)]
    pub explorer: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CrosschainConfig {
    pub root_delay_threshold_seconds: u64,
    pub hub_event_lookback_blocks: u64,
    pub hub_event_chunk_size: u64,
    pub verifier_event_lookback_blocks: u64,
    pub verifier_event_chunk_size: u64,
}

impl Default for CrosschainConfig {
    fn default() -> Self {
        Self {
            root_delay_threshold_seconds: 2400,
            hub_event_lookback_blocks: 50_000,
            hub_event_chunk_size: 10_000,
            verifier_event_lookback_blocks: 500_000,
            verifier_event_chunk_size: 10_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexerConfig {
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_cycles: u32,
}

#[derive(Debug, Deserialize)]
pub struct AlertConfig {
    #[serde(default = "default_cooldown")]
    pub cooldown_seconds: u64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            cooldown_seconds: default_cooldown(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IcpCanisterConfig {
    pub name: String,
    pub canister_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IcpConfig {
    pub replica_url: String,
    pub cycle_threshold: u64,
    pub canisters: Vec<IcpCanisterConfig>,
}

impl Default for IcpConfig {
    fn default() -> Self {
        Self {
            replica_url: "https://ic0.app".to_string(),
            cycle_threshold: 5_000_000_000_000,
            canisters: vec![],
        }
    }
}

fn default_interval() -> u64 {
    60
}

fn default_stale_threshold() -> u32 {
    5
}

fn default_cooldown() -> u64 {
    3600
}

pub fn load_config(path: impl AsRef<Path>) -> Result<WatcherConfig> {
    let path_ref = path.as_ref();
    let contents = std::fs::read_to_string(path_ref)
        .with_context(|| format!("failed to read config file {}", path_ref.display()))?;
    let expanded =
        expand_env_vars(&contents).context("failed to expand environment variables in config")?;
    let config: WatcherConfig =
        serde_yaml::from_str(&expanded).context("failed to parse watcher YAML config")?;
    Ok(config)
}
