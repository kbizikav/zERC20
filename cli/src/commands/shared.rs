use std::str::FromStr;

use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail};
use candid::Principal;
use client_common::{
    contracts::{hub::HubContract, verifier::VerifierContract, z_erc20::ZErc20Contract},
    tokens::{HubEntry, TokenEntry},
};
use hex;
use ic_agent::{Agent, identity::AnonymousIdentity};
use num_bigint::BigUint;
use stealth_client::client::StealthCanisterClient;

use crate::CommonArgs;

pub fn find_token_by_chain<'a>(tokens: &'a [TokenEntry], chain_id: u64) -> Result<&'a TokenEntry> {
    let mut matches = tokens.iter().filter(|token| token.chain_id == chain_id);
    match (matches.next(), matches.next()) {
        (None, _) => bail!("no tokens configured for chain id {}", chain_id,),
        (Some(_), Some(_)) => bail!(
            "multiple tokens configured for chain id {} — please disambiguate by label",
            chain_id,
        ),
        (Some(entry), None) => Ok(entry),
    }
}

pub fn parse_address(value: &str) -> Result<Address, String> {
    Address::from_str(value).map_err(|err| err.to_string())
}

pub fn parse_b256(value: &str) -> Result<B256, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(B256::ZERO);
    }

    let bytes = if let Some(hex_str) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        let mut normalized = hex_str.to_string();
        if normalized.len() % 2 == 1 {
            normalized.insert(0, '0');
        }
        hex::decode(&normalized).map_err(|err| err.to_string())?
    } else {
        let bigint = BigUint::from_str(trimmed).map_err(|err| err.to_string())?;
        bigint.to_bytes_be()
    };

    if bytes.len() > 32 {
        return Err("value does not fit into 256 bits".to_string());
    }
    let mut padded = [0u8; 32];
    let start = 32 - bytes.len();
    padded[start..].copy_from_slice(&bytes);
    Ok(B256::from(padded))
}

pub fn parse_u256(value: &str) -> Result<U256, String> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        parse_hex_to_u256(hex)
    } else {
        let bigint = BigUint::from_str(trimmed).map_err(|err| err.to_string())?;
        bigint_to_u256(&bigint)
    }
}

fn parse_hex_to_u256(hex_str: &str) -> Result<U256, String> {
    if hex_str.is_empty() {
        return Ok(U256::ZERO);
    }
    let bytes = hex::decode(hex_str).map_err(|err| err.to_string())?;
    if bytes.len() > 32 {
        return Err("hex value does not fit into 256 bits".to_string());
    }
    Ok(U256::from_be_bytes(fill_to_32(bytes)))
}

fn bigint_to_u256(value: &BigUint) -> Result<U256, String> {
    let bytes = value.to_bytes_be();
    if bytes.len() > 32 {
        return Err("decimal value does not fit into 256 bits".to_string());
    }
    Ok(U256::from_be_bytes(fill_to_32(bytes)))
}

fn fill_to_32(mut bytes: Vec<u8>) -> [u8; 32] {
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.append(&mut bytes);
        bytes = padded;
    }
    let mut array = [0u8; 32];
    array.copy_from_slice(&bytes);
    array
}

pub fn format_tx_hash(hash: &[u8]) -> String {
    format!("0x{}", hex::encode(hash))
}

pub fn build_erc20(entry: &TokenEntry) -> Result<ZErc20Contract> {
    let provider = entry.provider()?;
    Ok(ZErc20Contract::new(provider, entry.token_address).with_legacy_tx(entry.legacy_tx))
}

pub fn build_verifier(entry: &TokenEntry) -> Result<VerifierContract> {
    let provider = entry.provider()?;
    Ok(VerifierContract::new(provider, entry.verifier_address).with_legacy_tx(entry.legacy_tx))
}

pub fn build_hub(entry: &HubEntry) -> Result<HubContract> {
    let provider = entry.provider()?;
    Ok(HubContract::new(provider, entry.hub_address))
}

pub async fn build_stealth_client(common: &CommonArgs) -> Result<StealthCanisterClient> {
    let agent = Agent::builder()
        .with_url(common.ic_replica_url.clone())
        .with_identity(AnonymousIdentity)
        .build()
        .context("failed to build IC agent")?;

    if is_local_replica(&common.ic_replica_url) {
        agent
            .fetch_root_key()
            .await
            .context("failed to fetch replica root key")?;
    }

    let storage_id = Principal::from_text(&common.storage_canister_id)
        .context("failed to parse storage canister principal")?;
    let key_manager_id = Principal::from_text(&common.key_manager_canister_id)
        .context("failed to parse key manager canister principal")?;

    Ok(StealthCanisterClient::new(
        agent,
        storage_id,
        key_manager_id,
    ))
}

fn is_local_replica(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost")
}
