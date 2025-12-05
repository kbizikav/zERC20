use std::convert::TryFrom;

use alloy::primitives::{Address, U256};
use anyhow::{Result, anyhow, bail};

/// Parse a 20-byte address from raw bytes.
pub fn parse_address(bytes: &[u8]) -> Result<Address> {
    if bytes.len() != 20 {
        bail!("address bytes must be 20, got {}", bytes.len());
    }
    Ok(Address::from_slice(bytes))
}

/// Parse a big-endian 32-byte U256 from raw bytes.
pub fn parse_u256(bytes: &[u8]) -> Result<U256> {
    if bytes.len() != 32 {
        bail!("value bytes must be 32, got {}", bytes.len());
    }
    Ok(U256::from_be_slice(bytes))
}

/// Convert u64 to i64 with a labeled overflow error.
pub fn u64_to_i64(label: &str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        anyhow!(
            "{label} exceeds i64 range: {value}",
            label = label,
            value = value
        )
    })
}
