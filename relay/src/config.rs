use alloy::primitives::U256;
use anyhow::{Context, Result};
use client_common::tokens::{TokenEntry, load_tokens_from_path};
use std::path::PathBuf;

/// Top-level relay node configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub port: u16,
    pub private_key: String,
    pub tokens: Vec<TokenEntry>,
    /// Whether the swap endpoint is enabled. Default: false.
    pub swap_enabled: bool,
    /// Swap fee in basis points (100 = 1%). Default: 50 (0.5%).
    pub swap_fee_bps: u64,
    /// Maximum native token (wei) the relay will pay per swap. Default: 0.0001 ETH.
    pub max_swap_native_wei: U256,
}

impl RelayConfig {
    /// Load configuration from environment variables and tokens file.
    pub fn from_env() -> Result<Self> {
        let port: u16 = std::env::var("RELAY_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .context("RELAY_PORT must be a valid u16")?;

        let private_key =
            std::env::var("RELAY_PRIVATE_KEY").context("RELAY_PRIVATE_KEY is required")?;

        let tokens_path: PathBuf = std::env::var("TOKENS_FILE_PATH")
            .unwrap_or_else(|_| "../config/tokens.json".to_string())
            .into();
        let tokens_file =
            load_tokens_from_path(&tokens_path).context("failed to load tokens config")?;

        let swap_enabled = std::env::var("SWAP_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let swap_fee_bps: u64 = std::env::var("SWAP_FEE_BPS")
            .unwrap_or_else(|_| "50".to_string())
            .parse()
            .context("SWAP_FEE_BPS must be a valid u64")?;

        // Default: 0.0001 ETH = 100_000_000_000_000 wei
        let max_swap_native_wei = match std::env::var("MAX_SWAP_NATIVE_WEI") {
            Ok(v) => U256::from_str_radix(&v, 10)
                .context("MAX_SWAP_NATIVE_WEI must be a valid decimal U256")?,
            Err(_) => U256::from(100_000_000_000_000u64),
        };

        Ok(Self {
            port,
            private_key,
            tokens: tokens_file.tokens,
            swap_enabled,
            swap_fee_bps,
            max_swap_native_wei,
        })
    }
}
