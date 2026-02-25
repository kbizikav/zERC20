# fee-manager

Periodic maintenance worker that dynamically adjusts `targetLiquidity` on all
`LiquidityManager` contracts across chains to ensure balanced liquidity
distribution.

## Overview

The fee-manager calculates the optimal `targetLiquidity` for each chain based on
the total underlying token liquidity across all chains:

```
targetLiquidity = total_underlying_balance / number_of_chains * target_ratio_bps / 10000
```

This ensures that:

- Liquidity is encouraged to be evenly distributed across chains
- Fee/reward incentives automatically adjust as liquidity flows between chains
- Manual parameter tuning is eliminated

## Prerequisites

- Rust nightly toolchain (edition 2024 requires nightly) and `cargo` available in `PATH`.
  Use `cargo +nightly run -p fee-manager` or set nightly as default with `rustup default nightly`.
- Access to RPC endpoints for every chain with a LiquidityManager, listed in
  `../config/tokens.json` or a user-provided file with the same schema.
- A funded Ethereum-compatible private key with `FEE_MANAGER_ROLE` permission
  on each `LiquidityManager` contract.

## Configuration

Create a copy of `.env.example` and tailor the values:

```bash
cp fee-manager/.env.example fee-manager/.env
```

### Environment Variables

| Variable                    | Required | Default                 | Description                                             |
| --------------------------- | -------- | ----------------------- | ------------------------------------------------------- |
| `FEE_MANAGER_PRIVATE_KEY`   | Yes      | -                       | Hex-encoded 32-byte private key with `FEE_MANAGER_ROLE` |
| `TOKENS_FILE_PATH`          | No       | `../config/tokens.json` | Path to token metadata file                             |
| `FEE_MANAGER_INTERVAL_SECS` | No       | `3600`                  | Interval between fee parameter updates (seconds)        |
| `FEE_MANAGER_K_BPS`         | No       | `1000`                  | Incentive coefficient k in basis points (1 = 0.01%)     |
| `FEE_MANAGER_TARGET_RATIO_BPS` | No   | `8000`                  | Target liquidity ratio in basis points (8000 = 80%)     |
| `RUST_LOG`                  | No       | `info`                  | Log level (trace, debug, info, warn, error)             |

### tokens.json Requirements

Each token entry must include:

- `label`: Human-readable chain name (for logging)
- `token_address`: Address of the zERC20 token contract
- `verifier_address`: Address of the Verifier contract
- `liquidity_manager_address`: Address of the LiquidityManager contract
- `chain_id`: Chain identifier
- `deployed_block_number`: Block number when the contract was deployed (required)
- `rpc_urls`: Array of RPC endpoints (primary + fallbacks)
- `legacy_tx`: (optional) Set to `true` for chains requiring legacy transactions (e.g., BNB)

Example entry:

```json
{
  "label": "arb-sepolia-eth",
  "token_address": "0x64007Dd4818A530FDD3580341F02354e596772C6",
  "verifier_address": "0x0BD8923125B2c6A0093723f66D4B1EEa75aA0c5E",
  "liquidity_manager_address": "0xBDE0a0929388865C6b6f883513e9bbe38CfBb46c",
  "chain_id": 421614,
  "deployed_block_number": 10097248,
  "rpc_urls": ["https://sepolia-rollup.arbitrum.io/rpc"],
  "legacy_tx": false
}
```

## Running

### Local Development

From the repository root (requires Rust nightly for edition 2024):

```bash
cargo +nightly run -p fee-manager
```

### Single Execution (Smoke Test)

Use `--once` flag to run the update logic once and exit:

```bash
cargo +nightly run -p fee-manager -- --once
```

Example with explicit config file:

```bash
RUST_LOG=info \
FEE_MANAGER_PRIVATE_KEY=0x... \
TOKENS_FILE_PATH=config/tokens.zETH.json \
cargo +nightly run -p fee-manager -- --once
```

### Docker

The fee-manager is included in the docker-compose stack:

```bash
docker compose up fee-manager
```

Or run the full stack:

```bash
docker compose up -d
```

## How It Works

1. **Fetch Balances**: Query underlying token balance held by each
   `LiquidityManager` contract across all configured chains.

2. **Calculate Target**: Compute `targetLiquidity = total_balance / chain_count * target_ratio_bps / 10000`.

3. **Update Parameters**: For each chain, call `setFeeParams({ targetLiquidity, k })`
   on the `LiquidityManager` contract.

4. **Sleep**: Wait for the configured interval before repeating.

## Native ETH Support

The fee-manager automatically detects whether a LiquidityManager uses native ETH
or an ERC20 token as the underlying asset:

- **Native ETH**: Detected via the ERC-7528 sentinel address `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`.
  Balance is fetched using `provider.get_balance()`.
- **ERC20 tokens**: Balance is fetched using `balanceOf()` on the underlying token contract.

The underlying token address is read from each `LiquidityManager.underlyingToken()` at startup.

## FeeParams Structure

```javascript
struct FeeParams {
    uint256 targetLiquidity;  // Target liquidity where incentives fade to zero
    uint256 k;                // Incentive strength coefficient (basis points)
}
```

- `targetLiquidity`: The liquidity level at which wrap rewards and unwrap fees
  approach zero. Below this level, wrapping is rewarded; above, unwrapping is
  cheaper.
- `k`: Controls the steepness of the incentive curve. Higher values mean
  stronger incentives when liquidity deviates from target.

## Permissions

The private key used must have `FEE_MANAGER_ROLE` on each `LiquidityManager`.
This role can be granted by the contract owner:

```javascript
liquidityManager.grantRole(FEE_MANAGER_ROLE, feeManagerAddress);
```

## Logging

The fee-manager logs:

- Current underlying balance per chain
- Calculated total liquidity and per-chain target
- Transaction hashes for each `setFeeParams` call
- Any errors encountered during execution

Set `RUST_LOG=debug` for verbose output including RPC calls.
