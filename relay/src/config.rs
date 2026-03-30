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
    /// RPC URL used to query Chainlink price feeds (Ethereum mainnet).
    /// Falls back to the RPC of any configured mainnet-like chain if unset.
    pub oracle_rpc_url: Option<String>,
    /// Whether the swap endpoint is enabled. Default: false.
    pub swap_enabled: bool,
    /// Swap fee in basis points (100 = 1%). Default: 50 (0.5%).
    pub swap_fee_bps: u64,
    /// Maximum native token (wei) the relay will pay per swap. Default: 0.01 ETH.
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
            std::env::var("RELAYER_PRIVATE_KEY").context("RELAYER_PRIVATE_KEY is required")?;

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
        anyhow::ensure!(
            swap_fee_bps < 10_000,
            "SWAP_FEE_BPS must be < 10000 (got {})",
            swap_fee_bps
        );

        // Default: 0.01 ETH = 10_000_000_000_000_000 wei
        let max_swap_native_wei = match std::env::var("MAX_SWAP_NATIVE_WEI") {
            Ok(v) => U256::from_str_radix(&v, 10)
                .context("MAX_SWAP_NATIVE_WEI must be a valid decimal U256")?,
            Err(_) => U256::from(10_000_000_000_000_000u64),
        };
        anyhow::ensure!(
            !max_swap_native_wei.is_zero(),
            "MAX_SWAP_NATIVE_WEI must be non-zero"
        );

        let oracle_rpc_url = std::env::var("ORACLE_RPC_URL").ok();

        Ok(Self {
            port,
            private_key,
            tokens: tokens_file.tokens,
            oracle_rpc_url,
            swap_enabled,
            swap_fee_bps,
            max_swap_native_wei,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_test_tokens_file() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("zerc20-relay-config-{unique}.json"));
        let json = r#"{
          "tokens": [
            {
              "label": "zUSDC",
              "token_address": "0x0000000000000000000000000000000000000001",
              "verifier_address": "0x0000000000000000000000000000000000000002",
              "chain_id": 1,
              "deployed_block_number": 1,
              "rpc_urls": ["http://localhost:8545"]
            }
          ]
        }"#;
        fs::write(&path, json).expect("write tokens file");
        path
    }

    fn set_base_env(tokens_path: &PathBuf) {
        unsafe {
            std::env::set_var(
                "RELAYER_PRIVATE_KEY",
                "0x1111111111111111111111111111111111111111111111111111111111111111",
            );
            std::env::set_var("TOKENS_FILE_PATH", tokens_path);
            std::env::remove_var("SWAP_ENABLED");
            std::env::remove_var("SWAP_FEE_BPS");
            std::env::remove_var("MAX_SWAP_NATIVE_WEI");
            std::env::remove_var("RELAY_PORT");
        }
    }

    #[test]
    fn from_env_rejects_swap_fee_bps_at_10000() {
        let _guard = env_lock().lock().expect("env lock");
        let tokens_path = write_test_tokens_file();
        set_base_env(&tokens_path);
        unsafe {
            std::env::set_var("SWAP_FEE_BPS", "10000");
        }

        let err = RelayConfig::from_env().expect_err("should reject 10000 bps");
        assert!(err.to_string().contains("SWAP_FEE_BPS must be < 10000"));

        fs::remove_file(tokens_path).ok();
    }

    #[test]
    fn from_env_rejects_zero_max_swap_native_wei() {
        let _guard = env_lock().lock().expect("env lock");
        let tokens_path = write_test_tokens_file();
        set_base_env(&tokens_path);
        unsafe {
            std::env::set_var("MAX_SWAP_NATIVE_WEI", "0");
        }

        let err = RelayConfig::from_env().expect_err("should reject zero max native");
        assert!(
            err.to_string()
                .contains("MAX_SWAP_NATIVE_WEI must be non-zero")
        );

        fs::remove_file(tokens_path).ok();
    }
}
