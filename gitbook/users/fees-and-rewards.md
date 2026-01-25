# Fees and Rewards

This page explains how wrap/unwrap fees and rewards work in zERC20.

## Overview

zERC20 uses a dynamic fee system to maintain balanced liquidity across all supported chains. When liquidity on a chain deviates from the target level, the system applies incentives to encourage rebalancing:

| Action | Low Liquidity Chain | High Liquidity Chain |
|--------|---------------------|----------------------|
| **Wrap** | Earn rewards | No reward |
| **Unwrap** | Pay fee | No fee |

## Why Fees and Rewards Exist

Each chain holds a pool of underlying tokens (USDC, ETH, etc.) that back the zERC20 supply. When liquidity becomes unevenly distributed—for example, most USDC ends up on Ethereum while Arbitrum runs low—users on Arbitrum may face difficulty unwrapping.

The fee/reward system solves this by:

- **Rewarding deposits** on chains with low liquidity (encouraging users to add liquidity where it's needed)
- **Charging fees** on withdrawals from chains with low liquidity (discouraging draining of scarce liquidity)

## How Fees Are Calculated

Fees and rewards are calculated using a linear incentive curve based on two parameters:

- **Target Liquidity (T)**: The ideal liquidity level for the chain
- **Incentive Strength (k)**: How strongly the system incentivizes rebalancing

### Current Fee Settings

| Token | Incentive Strength (k) | Notes |
|-------|------------------------|-------|
| zUSDC | 10% | Fees/rewards apply across all supported chains |
| zETH | 10% | Fees/rewards apply across all supported chains |
| zBNB | — | No fees or rewards (single-chain only) |

> **Note**: zBNB only supports wrap/unwrap on BNB Chain, so there is no cross-chain liquidity balancing and therefore no fees or rewards.

**Target Liquidity (T)** is automatically calculated by a background service:

```
Target = Total liquidity across all chains / Number of chains
```

This ensures each chain's target stays aligned with actual liquidity distribution. As liquidity flows between chains, targets adjust automatically.

### The Formula

The fee/reward rate increases as liquidity drops further below the target:

```
When liquidity L < target T:
  Rate at point x = k × (1 - x/T)

When liquidity L ≥ target T:
  Rate = 0 (no fees or rewards)
```

The actual fee or reward is the integral (area under the curve) between your transaction's start and end liquidity levels.

### Example

Suppose a chain has:
- Current liquidity: 50,000 USDC
- Target liquidity: 100,000 USDC
- Incentive strength: 10%

**Unwrap 10,000 USDC**: You would pay a fee because liquidity is below target. The fee depends on how far below target the liquidity is.

**Wrap 10,000 USDC**: You would earn a reward for adding liquidity to an under-supplied chain.

## Fee Accumulation

Fees collected from unwraps accumulate in a surplus pool. This surplus is used to pay rewards for future wraps. When the surplus is depleted, wrap rewards are reduced accordingly.

## Cross-Chain Unwrap to Reduce Fees

If a chain has low liquidity and high unwrap fees, you can use **cross-chain unwrap** to access liquidity from a different chain with lower fees.

### How It Works

1. You initiate an unwrap on Chain A (where you hold zERC20)
2. Your zERC20 is bridged to Chain B (where liquidity is higher)
3. The unwrap happens on Chain B with lower fees
4. The underlying tokens are bridged back to Chain A

This entire process happens in a single transaction using LayerZero and Stargate.

### When to Use Cross-Chain Unwrap

- The unwrap fee on your current chain is high
- Another chain has more liquidity (lower fees)
- The bridge costs are less than the fee savings

The frontend displays both options with fee estimates, so you can compare before confirming.

## Viewing Current Fees

In the frontend:

1. Open the **Wrap / Unwrap** modal
2. Select the **UNWRAP** tab
3. Enter an amount

The interface shows:
- **Same Chain**: Fee amount for unwrapping on the current chain
- **Different Chain**: Fee amounts for cross-chain unwrap options

Compare these to find the most cost-effective option.

## Related

- [Getting Started](frontend/getting-started.md) — Basic wrap/unwrap instructions
- [Contract Spec: IncentiveLib](../developers/specs/contract-spec.md#incentive-curve) — Technical details of the fee curve
