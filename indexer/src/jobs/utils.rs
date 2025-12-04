use alloy::primitives::{Address, U256};
use anyhow::{Result, bail};

pub fn parse_address(bytes: &[u8]) -> Result<Address> {
    if bytes.len() != 20 {
        bail!("address bytes must be 20, got {}", bytes.len());
    }
    Ok(Address::from_slice(bytes))
}

pub fn parse_u256(bytes: &[u8]) -> Result<U256> {
    if bytes.len() != 32 {
        bail!("value bytes must be 32, got {}", bytes.len());
    }
    Ok(U256::from_be_slice(bytes))
}
