// SPDX-License-Identifier: BUSL-1.1

use std::{
    convert::TryInto,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use alloy::primitives::B256;
use anyhow::{Context, Result, anyhow};
use client_common::tokens::{TokenEntry, load_tokens_from_path};
use reqwest::Url;
use serde::Deserialize;

use crate::{
    events::{
        BLOCK_SPAN_RECOMMENDED, EventIndexerConfig, FORWARD_SCAN_OVERLAP_RECOMMENDED,
        REORG_CHECK_WINDOW_RECOMMENDED,
    },
    trees::{DbMerkleTreeConfig, HISTORY_WINDOW_RECOMMENDED},
};
use zkp::nova::constants::TRANSFER_TREE_HEIGHT;

const DEFAULT_EVENT_INTERVAL_MS: u64 = 5_000;
const DEFAULT_TREE_INTERVAL_MS: u64 = 2_000;
const DEFAULT_TREE_HEIGHT: u32 = TRANSFER_TREE_HEIGHT as u32;
const DEFAULT_TREE_BATCH_SIZE: usize = 128;
const DEFAULT_ROOT_INTERVAL_MS: u64 = 5_000;
const DEFAULT_ROOT_SUBMIT_INTERVAL_MS: u64 = 10_000;
const DEFAULT_DECIDER_PROVER_TIMEOUT_SECS: u64 = 120;
const DEFAULT_DECIDER_PROVER_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone)]
pub struct IndexerConfig {
    pub database_url: String,
    pub tokens: Vec<TokenEntry>,
    pub event_indexer: EventJobConfig,
    pub tree: TreeJobConfig,
    pub root: RootJobConfig,
}

impl IndexerConfig {
    pub fn load(tokens_path: impl AsRef<Path>) -> Result<Self> {
        let env = EnvSettings::from_env()?;
        let tokens = load_tokens(tokens_path)?;
        if tokens.is_empty() {
            return Err(anyhow!("at least one token entry must be configured"));
        }

        let event_indexer = EventJobConfig {
            interval_ms: env.event_interval_ms,
            block_span: env.event_block_span,
            forward_scan_overlap: env.event_forward_scan_overlap,
            reorg_check_window: env.event_reorg_check_window,
            request_delay_ms: env.event_request_delay_ms,
            retry_base_delay_ms: env.event_retry_base_delay_ms,
            retry_max_attempts: env.event_retry_max_attempts,
        };
        event_indexer
            .ensure_valid()
            .context("invalid event indexer configuration")?;

        let tree = TreeJobConfig {
            interval_ms: env.tree_interval_ms,
            height: default_tree_height(),
            history_window: env.tree_history_window,
            batch_size: env.tree_batch_size,
        };
        tree.ensure_valid().context("invalid tree configuration")?;

        let root = RootJobConfig::new(
            env.root_interval_ms,
            env.root_submit_interval_ms,
            env.root_history_window.unwrap_or(tree.history_window),
            env.decider_prover_timeout_secs,
            env.decider_prover_poll_interval_ms,
            env.decider_prover_url,
            env.root_submitter_private_key.as_deref(),
            env.root_submission_enabled,
            env.root_artifacts_dir,
        )
        .context("invalid root prover configuration")?;

        Ok(Self {
            database_url: env.database_url,
            tokens,
            event_indexer,
            tree,
            root,
        })
    }
}

#[derive(Debug, Deserialize)]
struct EnvSettings {
    database_url: String,
    #[serde(default = "default_event_interval_ms")]
    event_interval_ms: u64,
    #[serde(default = "default_block_span")]
    event_block_span: u64,
    #[serde(default = "default_forward_overlap")]
    event_forward_scan_overlap: u64,
    #[serde(default = "default_reorg_check_window")]
    event_reorg_check_window: u64,
    #[serde(default)]
    event_request_delay_ms: u64,
    #[serde(default = "default_retry_base_delay_ms")]
    event_retry_base_delay_ms: u64,
    #[serde(default = "default_retry_max_attempts")]
    event_retry_max_attempts: u32,
    #[serde(default = "default_tree_interval_ms")]
    tree_interval_ms: u64,
    #[serde(default = "default_history_window")]
    tree_history_window: u64,
    #[serde(default = "default_tree_batch_size")]
    tree_batch_size: usize,
    #[serde(default = "default_root_interval_ms")]
    root_interval_ms: u64,
    #[serde(default = "default_root_submit_interval_ms")]
    root_submit_interval_ms: u64,
    #[serde(default)]
    root_history_window: Option<u64>,
    #[serde(default = "default_decider_prover_timeout_secs")]
    decider_prover_timeout_secs: u64,
    #[serde(default = "default_decider_prover_poll_interval_ms")]
    decider_prover_poll_interval_ms: u64,
    #[serde(default = "default_decider_prover_url")]
    decider_prover_url: String,
    #[serde(default = "default_root_submission_enabled")]
    root_submission_enabled: bool,
    #[serde(default)]
    root_submitter_private_key: Option<String>,
    #[serde(default)]
    root_artifacts_dir: Option<String>,
}

impl EnvSettings {
    fn from_env() -> Result<Self> {
        envy::from_env::<Self>().context("failed to load indexer environment settings")
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct EventJobConfig {
    #[serde(default = "default_event_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_block_span")]
    pub block_span: u64,
    #[serde(default = "default_forward_overlap")]
    pub forward_scan_overlap: u64,
    #[serde(default = "default_reorg_check_window")]
    pub reorg_check_window: u64,
    #[serde(default)]
    pub request_delay_ms: u64,
    #[serde(default = "default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,
    #[serde(default = "default_retry_max_attempts")]
    pub retry_max_attempts: u32,
}

impl EventJobConfig {
    fn ensure_valid(&self) -> Result<()> {
        if self.interval_ms == 0 {
            return Err(anyhow!("event job interval must be positive"));
        }
        EventIndexerConfig::new(
            self.block_span,
            self.forward_scan_overlap,
            self.reorg_check_window,
        )
        .map(|config| config.with_request_delay_ms(self.request_delay_ms))
        .map(|config| {
            config.with_rate_limit_backoff(self.retry_base_delay_ms, self.retry_max_attempts)
        })
        .context("invalid event indexer configuration")?;
        Ok(())
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn build_indexer_config(&self) -> Result<EventIndexerConfig> {
        EventIndexerConfig::new(
            self.block_span,
            self.forward_scan_overlap,
            self.reorg_check_window,
        )
        .map(|config| config.with_request_delay_ms(self.request_delay_ms))
        .map(|config| {
            config.with_rate_limit_backoff(self.retry_base_delay_ms, self.retry_max_attempts)
        })
        .context("failed to construct EventIndexerConfig")
    }
}

impl Default for EventJobConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_event_interval_ms(),
            block_span: default_block_span(),
            forward_scan_overlap: default_forward_overlap(),
            reorg_check_window: default_reorg_check_window(),
            request_delay_ms: 0,
            retry_base_delay_ms: default_retry_base_delay_ms(),
            retry_max_attempts: default_retry_max_attempts(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TreeJobConfig {
    #[serde(default = "default_tree_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_tree_height")]
    pub height: u32,
    #[serde(default = "default_history_window")]
    pub history_window: u64,
    #[serde(default = "default_tree_batch_size")]
    pub batch_size: usize,
}

impl TreeJobConfig {
    fn ensure_valid(&self) -> Result<()> {
        if self.interval_ms == 0 {
            return Err(anyhow!("tree job interval must be positive"));
        }
        if self.height == 0 {
            return Err(anyhow!("tree height must be positive"));
        }
        if self.batch_size == 0 {
            return Err(anyhow!("tree job batch_size must be positive"));
        }
        DbMerkleTreeConfig::new(self.history_window)
            .context("invalid merkle tree configuration")?;
        Ok(())
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn build_tree_config(&self) -> Result<DbMerkleTreeConfig> {
        DbMerkleTreeConfig::new(self.history_window)
            .context("failed to construct DbMerkleTreeConfig")
    }
}

impl Default for TreeJobConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_tree_interval_ms(),
            height: default_tree_height(),
            history_window: default_history_window(),
            batch_size: default_tree_batch_size(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RootJobConfig {
    pub interval_ms: u64,
    pub submit_interval_ms: u64,
    pub history_window: u64,
    pub prover_timeout: Duration,
    pub prover_poll_interval: Duration,
    pub prover_url: Url,
    pub submitter_private_key: B256,
    pub submission_enabled: bool,
    pub artifacts_dir: PathBuf,
}

impl RootJobConfig {
    #[allow(clippy::too_many_arguments)]
    fn new(
        interval_ms: u64,
        submit_interval_ms: u64,
        history_window: u64,
        prover_timeout_secs: u64,
        prover_poll_interval_ms: u64,
        prover_url: String,
        submitter_private_key: Option<&str>,
        submission_enabled: bool,
        artifacts_dir: Option<String>,
    ) -> Result<Self> {
        if interval_ms == 0 {
            return Err(anyhow!("root prover job interval must be positive"));
        }
        if submit_interval_ms == 0 {
            return Err(anyhow!("root prover submission interval must be positive"));
        }
        if history_window == 0 {
            return Err(anyhow!(
                "root prover history window must be greater than zero"
            ));
        }
        if prover_poll_interval_ms == 0 {
            return Err(anyhow!(
                "root prover poll interval must be greater than zero"
            ));
        }

        let prover_url = Url::parse(prover_url.trim())
            .context("failed to parse decider prover URL from DECIDER_PROVER_URL")?;
        let submitter_private_key =
            parse_submitter_private_key(submitter_private_key, submission_enabled)?;
        let artifacts_dir = match artifacts_dir {
            Some(path) => {
                let normalized = PathBuf::from(path);
                if normalized.is_relative() {
                    workspace_root().join(normalized)
                } else {
                    normalized
                }
            }
            None => workspace_root().join("nova_artifacts"),
        };

        Ok(Self {
            interval_ms,
            submit_interval_ms,
            history_window,
            prover_timeout: Duration::from_secs(prover_timeout_secs),
            prover_poll_interval: Duration::from_millis(prover_poll_interval_ms),
            prover_url,
            submitter_private_key,
            submission_enabled,
            artifacts_dir,
        })
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn submit_interval(&self) -> Duration {
        Duration::from_millis(self.submit_interval_ms)
    }
}

fn default_event_interval_ms() -> u64 {
    DEFAULT_EVENT_INTERVAL_MS
}

fn default_block_span() -> u64 {
    BLOCK_SPAN_RECOMMENDED
}

fn default_forward_overlap() -> u64 {
    FORWARD_SCAN_OVERLAP_RECOMMENDED
}

fn default_reorg_check_window() -> u64 {
    REORG_CHECK_WINDOW_RECOMMENDED
}

fn default_retry_base_delay_ms() -> u64 {
    5_000
}

fn default_retry_max_attempts() -> u32 {
    5
}

fn default_tree_interval_ms() -> u64 {
    DEFAULT_TREE_INTERVAL_MS
}

fn default_tree_height() -> u32 {
    DEFAULT_TREE_HEIGHT
}

fn default_history_window() -> u64 {
    HISTORY_WINDOW_RECOMMENDED
}

fn default_tree_batch_size() -> usize {
    DEFAULT_TREE_BATCH_SIZE
}

fn default_root_interval_ms() -> u64 {
    DEFAULT_ROOT_INTERVAL_MS
}

fn default_root_submit_interval_ms() -> u64 {
    DEFAULT_ROOT_SUBMIT_INTERVAL_MS
}

fn default_decider_prover_url() -> String {
    "http://127.0.0.1:8081".to_string()
}

fn default_root_submission_enabled() -> bool {
    true
}

fn default_decider_prover_timeout_secs() -> u64 {
    DEFAULT_DECIDER_PROVER_TIMEOUT_SECS
}

fn default_decider_prover_poll_interval_ms() -> u64 {
    DEFAULT_DECIDER_PROVER_POLL_INTERVAL_MS
}

fn load_tokens(path: impl AsRef<Path>) -> Result<Vec<TokenEntry>> {
    let tokens_file = load_tokens_from_path(path)?;
    Ok(tokens_file.tokens)
}

fn parse_hex_b256(value: &str) -> Result<B256> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(anyhow!("hex string must not be empty"));
    }
    let hex = normalized.strip_prefix("0x").unwrap_or(normalized);
    if hex.len() != 64 {
        return Err(anyhow!(
            "hex string must be 32 bytes (64 hex characters), got {}",
            hex.len()
        ));
    }
    let bytes = hex::decode(hex).context("failed to decode hex string into bytes")?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("hex string must decode to exactly 32 bytes"))?;
    Ok(B256::from(arr))
}

fn parse_submitter_private_key(value: Option<&str>, submission_enabled: bool) -> Result<B256> {
    match (value, submission_enabled) {
        (Some(value), _) => parse_hex_b256(value).context("invalid ROOT_SUBMITTER_PRIVATE_KEY"),
        (None, true) => Err(anyhow!(
            "ROOT_SUBMITTER_PRIVATE_KEY is required when ROOT_SUBMISSION_ENABLED=true"
        )),
        (None, false) => Ok(B256::ZERO),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::parse_submitter_private_key;

    #[test]
    fn root_submitter_key_is_optional_when_submission_is_disabled() {
        assert!(parse_submitter_private_key(None, false).is_ok());
    }

    #[test]
    fn root_submitter_key_is_required_when_submission_is_enabled() {
        let error = parse_submitter_private_key(None, true).unwrap_err();
        assert!(error.to_string().contains("ROOT_SUBMITTER_PRIVATE_KEY"));
    }
}
