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
    pub accounts: Vec<AccountConfig>,
    #[serde(default)]
    pub chains: HashMap<String, ChainConfig>,

    #[serde(default)]
    pub indexer: Option<IndexerConfig>,

    #[serde(default)]
    pub crosschain: Option<CrosschainConfig>,

    #[serde(default)]
    pub alert: AlertConfig,

    #[serde(default)]
    pub stats_interval_seconds: Option<u64>,
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
pub struct IndexerConfig {
    pub status_urls: Vec<String>,
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold_cycles: u32,
    #[serde(default = "default_max_index_gap")]
    pub max_index_gap: u64,
}

#[derive(Debug, Deserialize)]
pub struct CrosschainConfig {
    pub token_config_paths: Vec<String>,
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

fn default_interval() -> u64 {
    60
}

fn default_stale_threshold() -> u32 {
    5
}

fn default_max_index_gap() -> u64 {
    100
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
