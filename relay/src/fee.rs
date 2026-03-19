use alloy::{
    primitives::U256,
    providers::Provider,
};
use anyhow::{Context, Result};

use crate::config::ChainConfig;

/// Estimate the relayer fee for a given chain.
///
/// Computes gas cost using current gas price and a conservative gas limit,
/// then returns the fee denominated in the token's smallest unit.
///
/// This is a simplified estimator — production deployments should factor in
/// the LiquidityManager unwrap fee and oracle-based token pricing.
pub async fn estimate_fee(
    provider: &impl Provider,
    _chain: &ChainConfig,
) -> Result<U256> {
    let gas_price = provider
        .get_gas_price()
        .await
        .context("failed to fetch gas price")?;

    // Conservative gas estimate for a teleport call.
    let gas_limit: u128 = 1_200_000;
    let gas_cost = U256::from(gas_price) * U256::from(gas_limit);

    // Apply 20% safety buffer.
    let fee = gas_cost + gas_cost / U256::from(5);
    Ok(fee)
}
