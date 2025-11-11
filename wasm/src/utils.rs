use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use ark_bn254::Fr;
use ark_ff::PrimeField;
use std::str::FromStr;
use wasm_bindgen::JsValue;

pub fn parse_u64(value: &str) -> Result<u64> {
    let normalized = value.trim();
    if normalized.is_empty() {
        anyhow::bail!("empty numeric value");
    }
    if let Some(hex) = normalized.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).context("invalid hex for u64")
    } else {
        normalized.parse::<u64>().context("invalid decimal for u64")
    }
}

pub fn hex_to_fr(value: &str) -> Result<Fr> {
    let normalized = value.trim();
    if normalized.is_empty() {
        anyhow::bail!("empty field element");
    }
    let hex = normalized.strip_prefix("0x").unwrap_or(normalized);
    let bytes = hex::decode(hex).context("invalid hex field element")?;
    Ok(Fr::from_be_bytes_mod_order(&bytes))
}

pub fn fr_to_hex(value: &Fr) -> String {
    use ark_ff::BigInteger;

    let mut bytes = value.into_bigint().to_bytes_be();
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    format!("0x{}", hex::encode(bytes))
}

pub fn anyhow_to_js_error(err: anyhow::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}

pub fn serde_error_to_js(err: serde_wasm_bindgen::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn log_timing(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn log_timing(_message: &str) {}

pub fn normalize_hex(value: &str) -> &str {
    value.strip_prefix("0x").unwrap_or(value)
}

pub fn parse_b256_hex(value: &str) -> Result<B256> {
    let hex = normalize_hex(value);
    let bytes = hex::decode(hex).context("invalid hex value")?;
    anyhow::ensure!(
        bytes.len() == 32,
        "expected 32-byte value, found {} bytes",
        bytes.len()
    );
    Ok(B256::from_slice(&bytes))
}

pub fn parse_address_hex(value: &str) -> Result<Address> {
    Address::from_str(value).context("invalid address")
}

pub fn b256_to_hex_string(value: B256) -> String {
    format!("0x{}", hex::encode(value.as_slice()))
}

pub fn address_to_hex_string(value: Address) -> String {
    format!("{:#x}", value)
}

pub fn fr_to_hex_checked(value: Fr) -> String {
    fr_to_hex(&value)
}

pub fn parse_u256_hex(value: &str) -> Result<U256> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty hex value");
    }
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    U256::from_str_radix(normalized, 16).context("invalid hex-encoded U256")
}

pub fn format_u256_hex(value: U256) -> String {
    format!("{value:#066x}")
}
