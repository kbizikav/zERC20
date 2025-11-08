use crate::utils::parse_address_hex;
use anyhow::Result;
use client_common::tokens::{HubEntry, TokenEntry};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsHubEntry {
    hub_address: String,
    chain_id: u64,
    #[serde(default)]
    rpc_urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsTokenEntry {
    label: String,
    token_address: String,
    verifier_address: String,
    #[serde(default)]
    minter_address: Option<String>,
    chain_id: u64,
    #[serde(default)]
    deployed_block_number: u64,
    #[serde(default)]
    rpc_urls: Vec<String>,
    #[serde(default)]
    legacy_tx: bool,
}

impl TryFrom<JsTokenEntry> for TokenEntry {
    type Error = anyhow::Error;

    fn try_from(value: JsTokenEntry) -> Result<Self> {
        Ok(TokenEntry {
            label: value.label,
            token_address: parse_address_hex(&value.token_address)?,
            verifier_address: parse_address_hex(&value.verifier_address)?,
            minter_address: value
                .minter_address
                .map(|addr| parse_address_hex(&addr))
                .transpose()?,
            chain_id: value.chain_id,
            deployed_block_number: value.deployed_block_number,
            rpc_urls: value.rpc_urls,
            legacy_tx: value.legacy_tx,
        })
    }
}

impl TryFrom<JsHubEntry> for HubEntry {
    type Error = anyhow::Error;

    fn try_from(value: JsHubEntry) -> Result<Self> {
        Ok(HubEntry {
            hub_address: parse_address_hex(&value.hub_address)?,
            chain_id: value.chain_id,
            rpc_urls: value.rpc_urls,
        })
    }
}
