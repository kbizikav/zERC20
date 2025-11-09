use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Read,
};

#[cfg(not(target_arch = "wasm32"))]
use std::{fs, path::Path};

use crate::contracts::utils::{NormalProvider, get_provider, get_provider_with_fallback};
use alloy::primitives::Address;
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::read::GzDecoder;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TokenEntry {
    pub label: String,
    pub token_address: Address,
    pub verifier_address: Address,
    #[serde(default)]
    pub minter_address: Option<Address>,
    pub chain_id: u64,
    pub deployed_block_number: u64,
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    #[serde(default)]
    pub legacy_tx: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HubEntry {
    pub hub_address: Address,
    pub chain_id: u64,
    #[serde(default)]
    pub rpc_urls: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct TokenMetadata {
    pub token_address: Address,
    pub verifier_address: Address,
    pub chain_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct TokensFile {
    #[serde(default)]
    pub hub: Option<HubEntry>,
    pub tokens: Vec<TokenEntry>,
}

impl TokenEntry {
    pub fn normalize(&mut self) -> Result<()> {
        if self.label.trim().is_empty() {
            return Err(anyhow!("token label must be non-empty"));
        }
        if self.rpc_urls.is_empty() {
            return Err(anyhow!(
                "token '{}' must configure at least one rpc url",
                self.label
            ));
        }
        Ok(())
    }

    pub fn metadata(&self) -> TokenMetadata {
        TokenMetadata {
            token_address: self.token_address,
            verifier_address: self.verifier_address,
            chain_id: self.chain_id,
        }
    }

    pub fn lock_key_with_salt(&self, salt: u64) -> i64 {
        let mut hasher = DefaultHasher::new();
        self.label.hash(&mut hasher);
        self.chain_id.hash(&mut hasher);
        self.token_address.hash(&mut hasher);
        self.verifier_address.hash(&mut hasher);
        salt.hash(&mut hasher);
        hasher.finish() as i64
    }

    pub fn provider(&self) -> Result<NormalProvider> {
        if self.rpc_urls.is_empty() {
            bail!("token '{}' has no rpc urls configured", self.label)
        }

        let provider = if cfg!(target_arch = "wasm32") {
            get_provider(self.rpc_urls[0].as_str())
        } else {
            get_provider_with_fallback(&self.rpc_urls)
        };
        provider.with_context(|| format!("failed to construct provider for '{}'", self.label))
    }

    pub const fn legacy_tx(&self) -> bool {
        self.legacy_tx
    }
}

impl HubEntry {
    pub fn normalize(&mut self) -> Result<()> {
        if self.rpc_urls.is_empty() {
            return Err(anyhow!("hub must configure at least one rpc url"));
        }
        Ok(())
    }

    pub fn provider(&self) -> Result<NormalProvider> {
        if self.rpc_urls.is_empty() {
            bail!("hub has no rpc urls configured")
        }
        let provider = if cfg!(target_arch = "wasm32") {
            get_provider(self.rpc_urls[0].as_str())
        } else {
            get_provider_with_fallback(&self.rpc_urls)
        };
        provider.with_context(|| "failed to construct provider for hub")
    }
}

impl TokensFile {
    pub fn normalize(&mut self) -> Result<()> {
        if let Some(hub) = self.hub.as_mut() {
            hub.normalize()?;
        }
        Ok(())
    }

    pub fn normalize_entries(&mut self) -> Result<()> {
        self.normalize()?;
        for token in self.tokens.iter_mut() {
            token
                .normalize()
                .with_context(|| format!("invalid token entry '{}'", token.label))?;
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_tokens_from_path(path: impl AsRef<Path>) -> Result<TokensFile> {
    let path_ref = path.as_ref();
    let contents = fs::read_to_string(path_ref)
        .with_context(|| format!("failed to read tokens config {}", path_ref.display()))?;
    parse_tokens_config(&contents)
        .with_context(|| format!("invalid tokens config {}", path_ref.display()))
}

pub fn load_tokens_from_compressed(payload: &str) -> Result<TokensFile> {
    let normalized: String = payload.chars().filter(|ch| !ch.is_whitespace()).collect();
    if normalized.is_empty() {
        return Err(anyhow!("TOKENS_COMPRESSED payload is empty"));
    }
    let decoded = STANDARD
        .decode(normalized.as_bytes())
        .context("failed to base64-decode TOKENS_COMPRESSED payload")?;
    let mut decoder = GzDecoder::new(decoded.as_slice());
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .context("failed to decompress TOKENS_COMPRESSED payload")?;
    parse_tokens_config(&json).context("invalid tokens payload from TOKENS_COMPRESSED")
}

pub fn parse_tokens_config(contents: &str) -> Result<TokensFile> {
    let mut file: TokensFile =
        serde_json::from_str(contents).context("failed to parse tokens config JSON")?;
    file.normalize_entries()
        .context("invalid tokens config entries")?;
    Ok(file)
}
