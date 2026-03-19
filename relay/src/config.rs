use alloy::primitives::Address;
use anyhow::{Context, Result};
use serde::Deserialize;

/// Per-chain configuration for the relay node.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub rpc_url: String,
    pub verifier_address: Address,
    pub token_address: Address,
    #[serde(default)]
    pub legacy_tx: bool,
}

/// Top-level relay node configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub port: u16,
    pub private_key: String,
    pub chains: Vec<ChainConfig>,
}

impl RelayConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self> {
        let port: u16 = std::env::var("RELAY_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .context("RELAY_PORT must be a valid u16")?;

        let private_key =
            std::env::var("RELAY_PRIVATE_KEY").context("RELAY_PRIVATE_KEY is required")?;

        let chains_json =
            std::env::var("RELAY_CHAINS").context("RELAY_CHAINS JSON array is required")?;
        let chains: Vec<ChainConfig> =
            serde_json::from_str(&chains_json).context("failed to parse RELAY_CHAINS JSON")?;

        Ok(Self {
            port,
            private_key,
            chains,
        })
    }
}
