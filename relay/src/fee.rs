use alloy::{primitives::U256, providers::Provider};
use anyhow::{Context, Result};
use client_common::tokens::TokenType;

use crate::oracle::PriceOracle;

/// Estimate the relayer fee for a given chain, denominated in the token's
/// smallest unit.
///
/// 1. Computes native gas cost (gas_price * gas_limit) with a 20% buffer.
/// 2. Converts from native wei to token units via the price oracle.
pub async fn estimate_fee(
    provider: &impl Provider,
    chain_id: u64,
    token_type: TokenType,
    oracle: &PriceOracle,
) -> Result<U256> {
    let gas_price = provider
        .get_gas_price()
        .await
        .context("failed to fetch gas price")?;

    // Conservative gas estimate for a teleport call.
    let gas_limit: u128 = 1_200_000;
    let gas_cost = U256::from(gas_price) * U256::from(gas_limit);

    // Apply 20% safety buffer.
    let native_fee = gas_cost + gas_cost / U256::from(5);

    // Convert native gas cost to token units.
    oracle
        .convert_native_to_token(chain_id, token_type, native_fee)
        .await
}
