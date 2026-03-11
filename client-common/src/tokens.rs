use std::{
    collections::hash_map::DefaultHasher,
    env,
    hash::{Hash, Hasher},
};

#[cfg(not(target_arch = "wasm32"))]
use std::{fs, path::Path};

use crate::contracts::utils::{NormalProvider, get_provider, get_provider_with_fallback};
use alloy::primitives::Address;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TokenEntry {
    pub label: String,
    pub token_address: Address,
    pub verifier_address: Address,
    #[serde(
        default,
        alias = "liquidityManagerAddress",
        alias = "minter_address",
        alias = "minterAddress"
    )]
    pub liquidity_manager_address: Option<Address>,
    #[serde(default, alias = "adaptor_address")]
    pub adaptor_address: Option<Address>,
    #[serde(default)]
    pub eid: Option<u32>,
    #[serde(default, alias = "layerzero_endpoint")]
    pub layerzero_endpoint: Option<Address>,
    pub chain_id: u64,
    pub deployed_block_number: u64,
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    #[serde(default)]
    pub legacy_tx: bool,
    #[serde(default, alias = "relayIntervalSecs", alias = "relay_interval_secs")]
    pub relay_interval_secs: Option<u64>,
    #[serde(
        default,
        alias = "rootSubmitIntervalMs",
        alias = "root_submit_interval_ms"
    )]
    pub root_submit_interval_ms: Option<u64>,
    #[serde(default, alias = "gelatoRelayAddress", alias = "gelato_relay_address")]
    pub gelato_relay_address: Option<Address>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HubEntry {
    pub hub_address: Address,
    pub chain_id: u64,
    #[serde(default)]
    pub eid: Option<u32>,
    #[serde(default, alias = "layerzeroEndpoint", alias = "layerZeroEndpoint")]
    pub layerzero_endpoint: Option<Address>,
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    #[serde(default, alias = "legacyTx")]
    pub legacy_tx: bool,
    #[serde(
        default,
        alias = "broadcastIntervalSecs",
        alias = "broadcast_interval_secs"
    )]
    pub broadcast_interval_secs: Option<u64>,
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
        if matches!(self.relay_interval_secs, Some(0)) {
            return Err(anyhow!(
                "token '{}' relay_interval_secs must be greater than zero",
                self.label
            ));
        }
        if matches!(self.root_submit_interval_ms, Some(0)) {
            return Err(anyhow!(
                "token '{}' root_submit_interval_ms must be greater than zero",
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
        if matches!(self.broadcast_interval_secs, Some(0)) {
            return Err(anyhow!(
                "hub broadcast_interval_secs must be greater than zero"
            ));
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

/// Expands environment variable placeholders in the format `${VAR}`.
///
/// Returns an error if a referenced environment variable is not defined.
pub fn expand_env_vars(contents: &str) -> Result<String> {
    let mut result = String::with_capacity(contents.len());
    let mut chars = contents.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(c) => var_name.push(c),
                    None => bail!("unclosed environment variable placeholder: ${{{var_name}"),
                }
            }
            // Support ${VAR:-default} syntax
            let (var_name, default_value) = match var_name.find(":-") {
                Some(pos) => (&var_name[..pos], Some(&var_name[pos + 2..])),
                None => (var_name.as_str(), None),
            };
            if var_name.is_empty() {
                bail!("empty environment variable name in placeholder");
            }
            match (env::var(var_name), default_value) {
                (Ok(value), _) => result.push_str(&value),
                (Err(_), Some(default)) => result.push_str(default),
                (Err(_), None) => {
                    bail!("environment variable '{var_name}' is not defined")
                }
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

pub fn parse_tokens_config(contents: &str) -> Result<TokensFile> {
    let expanded = expand_env_vars(contents).context("failed to expand environment variables")?;
    let mut file: TokensFile =
        serde_json::from_str(&expanded).context("failed to parse tokens config JSON")?;
    file.normalize_entries()
        .context("invalid tokens config entries")?;
    Ok(file)
}
