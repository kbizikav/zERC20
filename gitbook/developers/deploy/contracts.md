# Contract Deployment

This page covers every step needed to deploy and configure the on-chain components of a zERC20 token.

## Environment Setup

**Prerequisites:** Foundry installed, dependencies fetched via `forge soldeer install`.

Create a `.env` file (or export variables directly) using the template below:

```bash
PRIVATE_KEY=0x...
RPC_URL=https://base-sepolia.example
VERIFIER_RPC=https://arb-sepolia.example
DEPLOY_SALT=mytoken  # unique per token type to avoid address collisions
HUB_EID=40245
TOKEN_NAME=zUSDT
TOKEN_SYMBOL=zUSDT
TOKEN_DECIMALS=6
```

### Environment Variables Reference

| Variable | Required | Description |
|----------|----------|-------------|
| `PRIVATE_KEY` | Yes | Hex-encoded deployer private key |
| `RPC_URL` | Yes | RPC for Hub chain |
| `VERIFIER_RPC` | Yes | RPC for Verifier/Token chain |
| `DEPLOY_SALT` | Recommended | Unique salt per token type (avoids address collisions) |
| `HUB_EID` | Yes | LayerZero endpoint ID for Hub chain |
| `TOKEN_NAME` | Yes | ERC-20 token name |
| `TOKEN_SYMBOL` | Yes | ERC-20 token symbol |
| `TOKEN_DECIMALS` | No | Token decimals (default: 18, min: 6) |
| `VERIFIER_DELEGATE` | No | Verifier admin (defaults to deployer) |
| `TOKEN_OWNER` | No | Token owner (defaults to deployer) |
| `HUB_DELEGATE` | No | Hub admin (defaults to deployer) |

## Deploying the Hub

The Hub should be deployed on your "base" chain (e.g., Base). It aggregates transfer roots from all chains into a single global Merkle tree.

```bash
cd contracts
forge script script/DeployHub.s.sol:DeployHub \
  --rpc-url $RPC_URL --broadcast -vvvv
```

Record the deployed Hub address --- you will need it for peer wiring and token registration.

## Deploying Verifier and Token

Run on **each chain** where the token should exist:

```bash
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken \
  --rpc-url $VERIFIER_RPC --broadcast -vvvv
```

Record:

- Token proxy address
- Verifier proxy address

For multiple tokens (e.g., zUSDT and zWBTC on the same chain), use different `DEPLOY_SALT` values to avoid address collisions.

## Deploying LiquidityManager and Adaptor

The LiquidityManager handles wrapping/unwrapping the underlying ERC-20 into/from the zERC20 token. The Adaptor bridges liquidity across chains via Stargate.

**Required environment variables:**

- `ZERC20` --- token proxy address (from previous step)
- `LIQUIDITY_UNDERLYING_TOKEN` --- address of the underlying ERC-20 (e.g., USDT)

**Optional environment variables:**

- `LIQUIDITY_TARGET` --- target liquidity level
- `LIQUIDITY_K` --- fee curve parameter (basis points)
- `ADAPTOR_STARGATE` --- Stargate pool address
- `CHAIN_CONFIG_PATH` --- path to per-chain Stargate config

Per-chain Stargate config files are in `contracts/config/stargate/`:

- `config.zUSDC.json`
- `config.zETH.json`
- `config.zBNB.json`

Deploy:

```bash
export ZERC20=0x...
export LIQUIDITY_UNDERLYING_TOKEN=0x...
export CHAIN_CONFIG_PATH=contracts/config/stargate/config.zUSDC.json
forge script script/DeployLiquidity.s.sol:DeployLiquidity \
  --rpc-url $VERIFIER_RPC --broadcast -vvvv
```

## Wiring LayerZero Peers

Peer wiring connects the Hub, Verifiers, and Tokens across chains via LayerZero messaging. **Order matters:** Hub peers first, then Verifier peers, then Token peers.

### Step 1: SetHubPeers (on Hub chain)

```bash
export HUB_ADDRESS=0x...
export VERIFIER_ADDRESSES=0xVerA,0xVerB
export VERIFIER_EIDS=40231,40232
export TOKEN_ADDRESSES=0xTokA,0xTokB
export TOKEN_CHAIN_IDS=421614,11155420
forge script script/SetPeers.s.sol:SetHubPeers --rpc-url $RPC_URL --broadcast -vv
```

### Step 2: SetVerifierPeers (on each verifier chain)

```bash
export VERIFIER_ADDRESS=0x...
export HUB_EID=40245
forge script script/SetPeers.s.sol:SetVerifierPeers --rpc-url $VERIFIER_RPC --broadcast -vv
```

### Step 3: SetTokenPeers (on each token chain)

```bash
export TOKEN_ADDRESS=0x...
export PEER_ADDRESSES=0xTokB
export PEER_EIDS=40232
forge script script/SetPeers.s.sol:SetTokenPeers --rpc-url $VERIFIER_RPC --broadcast -vv
```

**Alternative:** use the Python helper for batch configuration:

```bash
./run_set_peers.py --file ../config/tokens.json -- --broadcast -vv
```

## Configuring DVN

DVN (Decentralized Verification Network) configuration controls which validators confirm cross-chain messages. Config files are in `contracts/config/dvn/`.

Use the helper script:

```bash
./run_set_dvn_config.py --config contracts/config/dvn/testnet/dvn-config.zusdc.testnet.json -- --broadcast -vv
```

## Registering Tokens on Hub

Each chain's Verifier and Token must be registered on the Hub so that the Hub recognizes incoming transfer roots.

```bash
cast send $HUB_ADDRESS \
  "registerToken((uint64,uint32,address,address))" \
  "($REMOTE_CHAIN_ID,$REMOTE_EID,$VERIFIER_ADDRESS,$TOKEN_ADDRESS)" \
  --rpc-url $HUB_RPC --private-key $PRIVATE_KEY
```

Repeat for every chain where the token is deployed.

## Creating tokens.json

Compile all deployed addresses into a config file that the off-chain infrastructure (indexer, crosschain job, etc.) will consume. The structure should match the following example (based on testnet config):

```json
{
  "tokens": [
    {
      "label": "arb-sepolia",
      "token_address": "0x...",
      "verifier_address": "0x...",
      "liquidity_manager_address": "0x...",
      "adaptor_address": "0x...",
      "chain_id": 421614,
      "deployed_block_number": 12345678,
      "eid": 40231,
      "layerzero_endpoint": "0x6EDCE65403992e310A62460808c4b910D972f10f",
      "rpc_urls": ["https://arb-sepolia.g.alchemy.com/v2/YOUR_KEY"],
      "root_submit_interval_ms": 100000,
      "relay_interval_secs": 900,
      "legacy_tx": false
    }
  ],
  "hub": {
    "hub_address": "0x...",
    "chain_id": 84532,
    "eid": 40245,
    "layerzero_endpoint": "0x6EDCE65403992e310A62460808c4b910D972f10f",
    "rpc_urls": ["https://base-sepolia.g.alchemy.com/v2/YOUR_KEY"],
    "broadcast_interval_secs": 1800,
    "legacy_tx": false
  }
}
```

Place this file in `config/deployed/testnet/` (or `config/deployed/mainnet/` for production) and reference it from infrastructure services.

## Contract Verification

Verify **implementation** contracts (not proxies) on block explorers:

```bash
forge verify-contract --chain-id <CHAIN_ID> --watch \
  <CONTRACT_ADDRESS> <PATH:CONTRACT> \
  --constructor-args <ABI_ENCODED_ARGS>
```

---

**See also:**

- [Overview](overview.md)
- [Infrastructure Setup](infrastructure.md)
- [End-to-End Walkthrough](end-to-end.md)
