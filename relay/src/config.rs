use anyhow::{Context, Result};
use client_common::tokens::{TokenEntry, load_tokens_from_path};
use std::path::PathBuf;

/// Top-level relay node configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub port: u16,
    pub private_key: String,
    pub tokens: Vec<TokenEntry>,
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

        Ok(Self {
            port,
            private_key,
            tokens: tokens_file.tokens,
        })
    }
}
