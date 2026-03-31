# Agent Notes (contracts/)

This directory is a Foundry project for deploying and testing the zERC20 contracts.

## Core contracts

- `src/Hub.sol`: Hub OApp (Base Sepolia in typical testnet setups)
- `src/Verifier.sol`: Verifier OApp (Arb/OP Sepolia)
- `src/zERC20.sol`: zERC20 token (OFT-core upgradeable)
- `src/relay/SwapHelper.sol`: Atomic permit + transferFrom + native payout helper for relay swaps
- `src/liquidity/LiquidityManager.sol`: Wrap/unwrap policy + fee/reward curve
- `src/liquidity/Adaptor.sol`: Unwrap + Stargate bridge + recovery accounting

## Scripts (most-used)

- `script/DeployHub.s.sol:DeployHub`
- `script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken`
- `script/DeploySwapHelper.s.sol:DeploySwapHelper`
- `script/DeployLiquidity.s.sol:DeployLiquidity`
- `script/SetPeers.s.sol:{SetHubPeers,SetVerifierPeers,SetTokenPeers}`
- `run_set_peers.py`: convenience runner reading `../config/tokens.json` (see caveat below)
- `run_set_dvn_config.py`: DVN/ULN config runner

## Chain config files

Per-chain configuration for LiquidityManager deployment (underlying token + Stargate):
- `config/config.zUSD.json` - USDC underlying
- `config/config.zETH.json` - Native ETH underlying (uses sentinel `0xEeee...eeee`)

## LayerZero IDs used in our common testnet setup

These EIDs are resolved via `lz-address-book` and are referenced in docs/scripts:

- Base Sepolia: `chainid=84532` → `EID=40245`
- Arbitrum Sepolia: `chainid=421614` → `EID=40231`
- Optimism Sepolia: `chainid=11155420` → `EID=40232`

## Recommended workflow (contracts-only smoke test)

Follow `contracts/README.md`. The typical order:

1) Deploy Hub (Base)
2) Deploy Verifier + zERC20 (Arb, OP)
3) Deploy LiquidityManager + Adaptor (Arb, OP)
4) Deploy `SwapHelper` on every chain where relay swaps should be enabled
5) Wire peers using direct forge commands (see below)
6) Smoke test: `wrap -> unwrapAndBridge` (Arb → OP is a common check)

For relay swaps, remember to write each deployed `SwapHelper` proxy address back into
`tokens.json` as `swap_helper_address`.

### Deploying multiple token types

When deploying a second token (e.g., zETH after zUSD), set a unique `DEPLOY_SALT` to avoid address collisions:
```bash
export DEPLOY_SALT=zETH
export CHAIN_CONFIG_PATH=config/config.zETH.json
```

### Peer configuration caveat

**`run_set_peers.py` may only simulate** even with `--broadcast` flag. If transactions save to
`broadcast/.../dry-run/` directories, use direct forge commands instead:

```bash
# SetHubPeers
export HUB_ADDRESS=0x... VERIFIER_ADDRESSES=0x...,0x... VERIFIER_EIDS=40231,40232 ...
forge script script/SetPeers.s.sol:SetHubPeers --rpc-url <RPC> --broadcast -vv

# SetVerifierPeers (per chain)
export HUB_ADDRESS=0x... HUB_EID=40245 VERIFIER_ADDRESS=0x...
forge script script/SetPeers.s.sol:SetVerifierPeers --rpc-url <RPC> --broadcast -vv

# SetTokenPeers (per chain)
export TOKEN_ADDRESS=0x... PEER_ADDRESSES=0x... PEER_EIDS=40232
forge script script/SetPeers.s.sol:SetTokenPeers --rpc-url <RPC> --broadcast -vv
```

## Troubleshooting notes (high signal)

- **`cast` output formatting**: `cast call` may print `10000000 [1e7]`. When exporting a value, strip it:
  `export X=$(cast call ... | awk '{print $1}')`
- **`extraOptions` vs tx gas**:
  - `extraOptions` controls destination LayerZero execution gas (e.g. `lzReceive` on the destination).
  - `cast send --gas-limit` controls the source-chain EVM transaction gas.
  - They are independent; raising one does not necessarily fix failures in the other.
- **Explorer shows “execution reverted” but tx success**:
  - `Adaptor.unwrapAndBridge` uses `try/catch` for the bridge leg; it can succeed overall while internal calls revert.
  - In that case, user funds are credited in adaptor storage and can be recovered via `Adaptor.withdraw(...)`.
- **`quoteFee` returns `(amount, 0, 0)`**:
  - Often means `tokenUnwrapFee >= amount` (commonly when LiquidityManager is under-collateralized).
  - Add liquidity (wrap more underlying) or try a larger amount.

## Verification (Etherscan-family)

- Verify **implementation** contracts (not proxies).
- Many explorers require Etherscan API v2; prefer:
  `forge verify-contract --chain-id <CHAIN_ID> --watch <ADDR> <PATH:NAME> --constructor-args <ABI_ENCODED>`
- Avoid forcing `--verifier-url` unless you know it’s the correct v2 endpoint for that explorer.

## Safety

- Never commit/paste real `PRIVATE_KEY` values or explorer API keys.
- Keep any `.env` files local; use placeholders in documentation.
