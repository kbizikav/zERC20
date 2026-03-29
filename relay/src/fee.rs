use alloy::{eips::BlockNumberOrTag, primitives::U256, providers::Provider};
use anyhow::{Context, Result};
use client_common::tokens::TokenType;

use crate::oracle::PriceOracle;

fn effective_gas_price(
    base_fee_per_gas: Option<u64>,
    priority_fee_per_gas: Option<u128>,
    legacy_gas_price: u128,
) -> U256 {
    match (base_fee_per_gas, priority_fee_per_gas) {
        (Some(base_fee), Some(priority_fee)) => U256::from(base_fee) + U256::from(priority_fee),
        _ => U256::from(legacy_gas_price),
    }
}

/// Gas limit assumed for a swap relay call.
pub const SWAP_GAS_LIMIT: u128 = 150_000;
/// Gas limit assumed for a redeem (teleport) relay call.
pub const REDEEM_GAS_LIMIT: u128 = 400_000;

/// Estimate the relayer's native gas cost in wei for a given `gas_limit`.
pub async fn estimate_native_fee(provider: &impl Provider, gas_limit: u128) -> Result<U256> {
    let gas_price_legacy = provider
        .get_gas_price()
        .await
        .context("failed to fetch gas price")?;

    let latest_block = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .context("failed to fetch latest block for fee estimation")?;
    let priority_fee = provider.get_max_priority_fee_per_gas().await.ok();
    let gas_price = effective_gas_price(
        latest_block.and_then(|block| block.header.base_fee_per_gas),
        priority_fee,
        gas_price_legacy,
    );

    let gas_cost = gas_price * U256::from(gas_limit);

    // Apply 20% safety buffer.
    Ok(gas_cost + gas_cost / U256::from(5))
}

/// Estimate the relayer fee for a given chain, denominated in the token's
/// smallest unit.
///
/// 1. Computes native gas cost using `baseFee + priorityFee` on EIP-1559
///    chains, or `gasPrice` on legacy chains, then applies a 20% buffer.
/// 2. Converts from native wei to token units via the price oracle.
pub async fn estimate_fee(
    provider: &impl Provider,
    chain_id: u64,
    token_type: TokenType,
    oracle: &PriceOracle,
) -> Result<U256> {
    let native_fee = estimate_native_fee(provider, REDEEM_GAS_LIMIT).await?;

    // Convert native gas cost to token units.
    oracle
        .convert_native_to_token(chain_id, token_type, native_fee)
        .await
}

#[cfg(test)]
mod tests {
    use super::effective_gas_price;
    use alloy::primitives::U256;

    #[test]
    fn uses_base_fee_plus_priority_fee_when_both_available() {
        let result = effective_gas_price(Some(100), Some(3), 999);
        assert_eq!(result, U256::from(103u64));
    }

    #[test]
    fn falls_back_to_legacy_gas_price_without_base_fee() {
        let result = effective_gas_price(None, Some(3), 999);
        assert_eq!(result, U256::from(999u64));
    }

    #[test]
    fn falls_back_to_legacy_gas_price_without_priority_fee() {
        let result = effective_gas_price(Some(100), None, 999);
        assert_eq!(result, U256::from(999u64));
    }
}
