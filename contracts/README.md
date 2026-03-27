Contracts Deployment Guide
==========================

This document explains how to deploy the LayerZero Hub, verifier, and `zERC20` token contracts that live in this directory. All deployment flows rely on Foundry scripts located under `script/`.

Prerequisites
-------------
- Foundry toolchain (`forge`, `cast`, `anvil`) installed via `foundryup`
- Soldeer-managed dependencies installed via `forge soldeer install`
- RPC endpoints for each network you intend to deploy to (for example Base Sepolia, Arbitrum Sepolia, Optimism Sepolia)
- A funded deployer key with permission to manage LayerZero configuration for the selected networks
- Endpoint IDs (EIDs) for every chain you plan to connect

Environment Variables
---------------------
The scripts consume environment variables through `vm.env*` helpers. Place the values in an `.env` file and load them with `source .env` before running any command.

### Shared
- `PRIVATE_KEY`: Hex-encoded private key for the broadcaster account (also used as the default delegate when overrides are omitted)
- `RPC_URL`: RPC endpoint that matches the target chain passed to `--rpc-url` (used for Hub deployment in the examples below)
- `VERIFIER_RPC`: RPC endpoint for the verifier/token chain (used only by the CLI flag in the example command)
- `DEPLOY_SALT` (string, optional): Overrides the base salt used for deterministic deployments.
  **Important**: When deploying multiple token types (e.g., zUSD and zETH), use different salts to avoid address collisions:
  ```bash
  export DEPLOY_SALT=zUSD   # for zUSD deployment
  export DEPLOY_SALT=zETH   # for zETH deployment
  ```
  If unset, all deployments use the same default salt and will collide.

### Hub deployment (`DeployHub`)
- `HUB_EID` (uint32): LayerZero endpoint ID for the chain hosting the Hub (for reference/logging)
- `HUB_DELEGATE` (address, optional): Account that will own the Hub and manage LayerZero config; defaults to the broadcaster wallet if omitted
  - The LayerZero endpoint address is resolved automatically from `lz-address-book` using `block.chainid` (ensure the chain ID is supported there).

### Verifier and token deployment (`DeployVerifierAndToken`)
- `TOKEN_NAME` (string): ERC20 token name
- `TOKEN_SYMBOL` (string): ERC20 token symbol
- `HUB_EID` (uint32): Hub endpoint identifier the verifier should target
- `VERIFIER_DELEGATE` (address, optional): Account that can update verifier LayerZero config; defaults to the broadcaster wallet if omitted
- `TOKEN_OWNER` (address, optional): Account that will own the token; defaults to the broadcaster wallet if omitted
- `TOKEN_DECIMALS` (uint, optional): Token decimals; defaults to `18` and must be at least `6`
  - The LayerZero endpoint address is resolved automatically from `lz-address-book` using `block.chainid` (ensure the chain ID is supported there).

### Sample `.env`
```bash
PRIVATE_KEY=0xabc123...
RPC_URL=https://base-sepolia.example
VERIFIER_RPC=https://optimism-sepolia.example
DEPLOY_SALT=my-optional-salt

# LayerZero V2 EIDs (from lz-address-book):
# - Base Sepolia (chainid 84532):        40245
# - Arbitrum Sepolia (chainid 421614):   40231
# - Optimism Sepolia (chainid 11155420): 40232
HUB_EID=40245
# HUB_DELEGATE=0xYourDelegate # optional; defaults to PRIVATE_KEY holder

TOKEN_NAME=zUSD
TOKEN_SYMBOL=zUSD
HUB_EID=40245
# TOKEN_DECIMALS=6 # e.g. zUSD/USDC-style tokens (LiquidityManager requires decimals match underlying)
# VERIFIER_DELEGATE=0xYourVerifierDelegate # optional; defaults to PRIVATE_KEY holder
# LIQUIDITY_MANAGER=0x0000000000000000000000000000000000000000

# Peer configuration scripts
# HUB_ADDRESS=0xHubOnThisChain
# VERIFIER_ADDRESSES=0xVerifierA,0xVerifierB
# VERIFIER_EIDS=40231,40232
# TOKEN_ADDRESSES=0xTokenA,0xTokenB
# TOKEN_CHAIN_IDS=421614,11155420
# VERIFIER_ADDRESS=0xVerifierOnThisChain
```

Pre-deploy Checks
-----------------
```bash
forge soldeer install
forge build
forge test
```
Run these commands inside `contracts/` to ensure the workspace compiles and tests pass before broadcasting transactions.

Deploying the Hub
-----------------
```bash
forge script script/DeployHub.s.sol:DeployHub \
  --rpc-url $RPC_URL \
  --broadcast \
  -vvvv
```
- Use the same `RPC_URL` chain that matches `HUB_EID`
- Add `--legacy` if the RPC only supports legacy gas pricing
- Pass `--etherscan-api-key <key>` to verify on the corresponding explorer, if supported

The script prints the deployed Hub address.

Deploying the Verifier and Token
--------------------------------
The `DeployVerifierAndToken` script now reads every parameter from environment variables. Ensure the required values listed above are exported (or loaded via `.env`) for the target chain, then run:
```bash
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken \
  --rpc-url $VERIFIER_RPC \
  --broadcast \
  -vvvv
```
The script logs the addresses of the token, verifier, and each deployed Nova decider contract and wires the verifier into the token automatically.

Deploying Liquidity Manager and Adaptor
---------------------------------------
The `DeployLiquidity` script deploys an upgradeable `LiquidityManager` and, when provided a Stargate address, a stateless `Adaptor` wired to that manager.

Purpose notes:
- `LiquidityManager` is the liquidity policy boundary for the system. It exists to keep the zERC20 supply anchored to real underlying liquidity while encoding the incentive curve that governs when liquidity should be attracted or released.
- `Adaptor` is the cross-chain exit and recovery boundary. It exists to turn zERC20 inflows into a controlled release of underlying value via Stargate while preserving user intent through slippage limits and refund accounting when bridging conditions change.

Required env:
- `ZERC20` (address): zERC20 token the manager mints/burns.
- `LIQUIDITY_UNDERLYING_TOKEN` (address): Underlying ERC20 held by the manager.
- `PRIVATE_KEY` (uint256): Broadcaster key.

Optional env (defaults shown in `script/DeployLiquidity.s.sol`):
- `LIQUIDITY_TARGET` (uint256): Target liquidity level that drives rewards/fees (defaults to 1_000_000e6).
- `LIQUIDITY_K` (uint256): Incentive strength coefficient for wrap rewards/unwrap fees, expressed in basis points (1 = 0.01%; 10_000 = 1.0). Defaults to `1_000` (you can set `0` to disable curve-based incentives).
- `LIQUIDITY_OWNER` (address): Admin/fee manager for the LiquidityManager (defaults to broadcaster).
- `ADAPTOR_STARGATE` (address): When set, deploys the Adaptor wired to this Stargate instance.
- Defaults can also be sourced from chain config files in `contracts/config/stargate/` (override with `CHAIN_CONFIG_PATH`), keyed by `block.chainid` with `underlyingToken` and `stargate` entries. Environment variables still take precedence for those values.
  - The LayerZero endpoint address is resolved automatically from `lz-address-book` using `block.chainid` (ensure the chain ID is supported there).

Example:
```bash
forge script script/DeployLiquidity.s.sol:DeployLiquidity \
  --rpc-url $VERIFIER_RPC \
  --broadcast \
  -vvvv
```

Testnet Walkthrough (Base Sepolia Hub + Arb/OP Sepolia Verifiers)
---------------------------------------------------------------
This is a minimal “audit pre-check” flow to ensure deployments, peer wiring, and the `Adaptor.unwrapAndBridge` path work end-to-end.

### Networks / IDs
- Base Sepolia: `chainid=84532`, LayerZero `EID=40245` (Hub)
- Arbitrum Sepolia: `chainid=421614`, LayerZero `EID=40231` (Verifier + zERC20)
- Optimism Sepolia: `chainid=11155420`, LayerZero `EID=40232` (Verifier + zERC20)

### 1) Deploy Hub (Base Sepolia)
```bash
cd contracts
export PRIVATE_KEY=0x...
export HUB_EID=40245
forge script script/DeployHub.s.sol:DeployHub --rpc-url <BASE_RPC> --broadcast -vvvv
```

### 2) Deploy Verifier + zERC20 (Arb Sepolia, then OP Sepolia)
For USDC-style tokens, set `TOKEN_DECIMALS=6` (LiquidityManager requires decimals match the underlying token).
```bash
export PRIVATE_KEY=0x...
export HUB_EID=40245
export TOKEN_NAME=zUSD
export TOKEN_SYMBOL=zUSD
export TOKEN_DECIMALS=6

forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url <ARB_RPC> --broadcast -vvvv
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url <OP_RPC> --broadcast -vvvv
```

**Deploying additional token types (e.g., zETH)**

To deploy a second token type without address collisions, set a unique `DEPLOY_SALT`:
```bash
export PRIVATE_KEY=0x...
export HUB_EID=40245
export TOKEN_NAME=zETH
export TOKEN_SYMBOL=zETH
export TOKEN_DECIMALS=18
export DEPLOY_SALT=zETH   # Different salt to avoid collisions with zUSD

forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url <ARB_RPC> --broadcast -vvvv
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url <OP_RPC> --broadcast -vvvv
```

### 3) Deploy LiquidityManager + Adaptor (Arb Sepolia, then OP Sepolia)
Use the shipped per-chain config files to select the underlying token and Stargate address:
- `contracts/config/stargate/config.zUSDC.json` (USDC)
- `contracts/config/stargate/config.zETH.json` (native ETH)
- `contracts/config/stargate/config.zBNB.json` (BNB)

```bash
export PRIVATE_KEY=0x...
export ZERC20=<zERC20 token proxy address>
export CHAIN_CONFIG_PATH=contracts/config/stargate/config.zUSDC.json

forge script script/DeployLiquidity.s.sol:DeployLiquidity --rpc-url <ARB_RPC> --broadcast -vvvv
forge script script/DeployLiquidity.s.sol:DeployLiquidity --rpc-url <OP_RPC> --broadcast -vvvv
```

### 4) Wire Hub/Verifier/Token peers

Use a tokens config file from `../config/` (e.g., `tokens.zusdc.testnet.json`, `tokens.zeth.testnet.json`, `tokens.zbnb.testnet.json`)
or copy `../config/tokens.example.json` to create your own, filling in `hub_address`, token/verifier addresses, `chain_id`, and `eid`.

**Option A: Direct forge commands (recommended)**

Run each script directly with forge for reliable broadcasting:
```bash
export PRIVATE_KEY=0x...

# SetHubPeers (on Hub chain)
export HUB_ADDRESS=0x... VERIFIER_ADDRESSES=0x...,0x... VERIFIER_EIDS=40231,40232 TOKEN_ADDRESSES=0x...,0x... TOKEN_CHAIN_IDS=421614,11155420
forge script script/SetPeers.s.sol:SetHubPeers --rpc-url <HUB_RPC> --broadcast -vv

# SetVerifierPeers (on each verifier chain)
export HUB_ADDRESS=0x... HUB_EID=40245 VERIFIER_ADDRESS=0x...
forge script script/SetPeers.s.sol:SetVerifierPeers --rpc-url <VERIFIER_RPC> --broadcast -vv

# SetTokenPeers (on each token chain)
export TOKEN_ADDRESS=0x... PEER_ADDRESSES=0x... PEER_EIDS=40232
forge script script/SetPeers.s.sol:SetTokenPeers --rpc-url <TOKEN_RPC> --broadcast -vv
```

**Option B: Python helper**

```bash
export PRIVATE_KEY=0x...
python3 ./run_set_peers.py --file ../config/tokens.zusdc.testnet.json -- --broadcast -vv
```

**Note**: The python helper may produce simulation-only output (saved to `dry-run/` directories). If transactions are not
broadcast, use Option A with direct forge commands instead.

Crosschain Unwrap Smoke Test (Adaptor.unwrapAndBridge)
------------------------------------------------------
This verifies `wrap -> unwrap -> Stargate sendToken -> destination receipt`.

### Known pitfalls
- `quoteFee(...)` returning `(amount, 0, 0)` means the unwrap fee is >= amount (commonly because the LiquidityManager is
  under-collateralized). Add more underlying liquidity (wrap more) before retrying.
- Ensure zERC20 allowance is set before calling `unwrapAndBridge`. If `allowance == 0`, the tx will revert early.
- `extraOptions` (destination execution gas) is different from the source-chain transaction `gasLimit`. If you suspect a
  source-chain gas issue, pass `cast send --gas-limit <N>` explicitly, but do not expect it to fix destination execution
  failures.
- Explorers/Tenderly may show “execution reverted” even when the tx is `success`; `Adaptor` uses `try/catch` and will
  keep funds credited inside the adaptor when bridging fails.
- When bridging fails, recover funds with `Adaptor.withdraw(...)` (see below).
- You do NOT need to pre-fund LiquidityManager on the destination chain for this smoke test: Stargate delivers the
  underlying directly to the destination receiver. (Stargate itself must have liquidity for the chosen asset.)
- Do not paste private keys or explorer API keys into shared logs/threads. Use environment variables.

### 1) Wrap underlying into zERC20 (source chain)
```bash
cast send <UNDERLYING> "approve(address,uint256)" <LIQUIDITY_MANAGER_PROXY> <AMOUNT> --rpc-url <SRC_RPC> --private-key $PRIVATE_KEY
cast send <LIQUIDITY_MANAGER_PROXY> "wrap(uint256,address)" <AMOUNT> <RECEIVER> --rpc-url <SRC_RPC> --private-key $PRIVATE_KEY
```

### 2) Approve adaptor + quote fees
`extraOptions` controls destination execution gas. A safe default for basic receipt is 500k LZ receive gas:
`0x0003010011010000000000000000000000000007a120` (gas=500,000).
```bash
export EXTRA=0x0003010011010000000000000000000000000007a120
export ZAMOUNT=$(cast call <ZERC20> "balanceOf(address)(uint256)" <RECEIVER> --rpc-url <SRC_RPC> | awk '{print $1}')
cast send <ZERC20> "approve(address,uint256)" <ADAPTOR_PROXY> $ZAMOUNT --rpc-url <SRC_RPC> --private-key $PRIVATE_KEY

cast call <ADAPTOR_PROXY> \
  "quoteFee(uint256,(uint32,address,uint256,bytes,bytes,bytes))((uint256,uint256,uint256))" \
  $ZAMOUNT "(<DST_EID>,<RECEIVER>,0,$EXTRA,0x,0x)" \
  --rpc-url <SRC_RPC>
```
Use the 2nd return value (`nativeBridgeFee`) as `--value` in the next step.

### 3) Unwrap and bridge
```bash
cast send <ADAPTOR_PROXY> \
  "unwrapAndBridge(uint256,(uint32,address,uint256,bytes,bytes,bytes))" \
  $ZAMOUNT "(<DST_EID>,<RECEIVER>,0,$EXTRA,0x,0x)" \
  --value <NATIVE_BRIDGE_FEE_WEI> \
  --rpc-url <SRC_RPC> \
  --private-key $PRIVATE_KEY
```

### 4) If the bridge fails, withdraw credited balances
```bash
cast call <ADAPTOR_PROXY> "underlyingTokenBalances(address)(uint256)" <RECEIVER> --rpc-url <SRC_RPC>
cast call <ADAPTOR_PROXY> "nativeBalances(address)(uint256)" <RECEIVER> --rpc-url <SRC_RPC>

cast send <ADAPTOR_PROXY> "withdraw(address,uint256)" <UNDERLYING> <AMOUNT> --rpc-url <SRC_RPC> --private-key $PRIVATE_KEY
cast send <ADAPTOR_PROXY> "withdraw(address,uint256)" 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE <WEI> --rpc-url <SRC_RPC> --private-key $PRIVATE_KEY
```

Contract Verification (Etherscan / Arbiscan / Basescan)
-------------------------------------------------------
Some explorers have migrated to Etherscan API v2. If verification fails with messages about deprecated v1 endpoints,
avoid forcing `--verifier-url` and rely on `--chain-id` routing:
```bash
forge verify-contract --chain-id <CHAIN_ID> --watch <CONTRACT_ADDRESS> <PATH:CONTRACT> --constructor-args <ABI_ENCODED_ARGS>
```

Notes:
- Verify the **implementation** contracts (not the proxies).
- Ensure `ETHERSCAN_API_KEY` (or pass `--etherscan-api-key`) is set in your shell.

Example (Adaptor implementation):
```bash
ARGS=$(cast abi-encode "constructor(address,address,address)" <LIQUIDITY_MANAGER_PROXY> <STARGATE> 0x6EDCE65403992e310A62460808c4b910D972f10f)
forge verify-contract --chain-id <CHAIN_ID> --watch <ADAPTOR_IMPL> src/liquidity/Adaptor.sol:Adaptor --constructor-args "$ARGS"
```

Other common implementation constructors:
```bash
# Hub (Base Sepolia)
ARGS=$(cast abi-encode "constructor(address)" 0x6EDCE65403992e310A62460808c4b910D972f10f)
forge verify-contract --chain-id 84532 --watch <HUB_IMPL> src/Hub.sol:Hub --constructor-args "$ARGS"

# Verifier (Arb/OP Sepolia)
ARGS=$(cast abi-encode "constructor(address)" 0x6EDCE65403992e310A62460808c4b910D972f10f)
forge verify-contract --chain-id 421614 --watch <VERIFIER_IMPL> src/Verifier.sol:Verifier --constructor-args "$ARGS"
forge verify-contract --chain-id 11155420 --watch <VERIFIER_IMPL> src/Verifier.sol:Verifier --constructor-args "$ARGS"

# zERC20 (Arb/OP Sepolia)
ARGS=$(cast abi-encode "constructor(address,uint8,address)" 0x6EDCE65403992e310A62460808c4b910D972f10f 6 <BLOCKLIST_ADDRESS>)
forge verify-contract --chain-id 421614 --watch <ZERC20_IMPL> src/zERC20.sol:zERC20 --constructor-args "$ARGS"
forge verify-contract --chain-id 11155420 --watch <ZERC20_IMPL> src/zERC20.sol:zERC20 --constructor-args "$ARGS"

# Blocklist (per chain)
ARGS=$(cast abi-encode "constructor(address)" <BLOCKLIST_OWNER>)
forge verify-contract --chain-id <CHAIN_ID> --watch <BLOCKLIST_ADDRESS> src/Blocklist.sol:Blocklist --constructor-args "$ARGS"

# LiquidityManager (per chain)
ARGS=$(cast abi-encode "constructor(address,address)" <UNDERLYING> <ZERC20_PROXY>)
forge verify-contract --chain-id 421614 --watch <LIQUIDITY_MANAGER_IMPL> src/liquidity/LiquidityManager.sol:LiquidityManager --constructor-args "$ARGS"
forge verify-contract --chain-id 11155420 --watch <LIQUIDITY_MANAGER_IMPL> src/liquidity/LiquidityManager.sol:LiquidityManager --constructor-args "$ARGS"
```

Deploying the Blocklist and Upgrading zERC20
---------------------------------------------
The `Blocklist` contract is a shared per-chain registry for OFAC-sanctioned addresses. It must be deployed before upgrading existing zERC20 proxies, as the zERC20 constructor now requires a `Blocklist` address as an immutable parameter.

### 1) Deploy Blocklist (once per chain)
```bash
export PRIVATE_KEY=0x...
export BLOCKLIST_OWNER=0xMultisigAddress  # optional; defaults to deployer

forge script script/DeployBlocklist.s.sol:DeployBlocklist \
  --rpc-url <RPC_URL> \
  --broadcast -vvvv
```
Note the deployed Blocklist address from the output.

### 2) Register sanctioned addresses
Call `blockAddresses(address[])` on the deployed Blocklist contract to register the OFAC list (~70 addresses).

### 3) Upgrade existing zERC20 proxies (per proxy)
```bash
export PRIVATE_KEY=0x...
export ZERC20_PROXY=0xProxyAddress
export BLOCKLIST_ADDRESS=0xBlocklistAddress

forge script script/upgrade/ZERC20BlocklistUpgrade.s.sol:UpgradeZERC20Blocklist \
  --rpc-url <RPC_URL> \
  --broadcast -vvvv
```
The script reads `endpoint` and `decimals` from the existing proxy, deploys a new implementation with the Blocklist immutable, and executes `upgradeToAndCall`. Repeat for each zERC20 proxy on each chain (4 chains × 2-3 tokens).

Verifier upgrade for relay fee support
--------------------------------------
If you upgrade an existing `Verifier` to the relay-fee version, you must initialize the new EIP-712 domain in the same transaction. A plain `upgradeTo(...)` leaves `initializeV2(...)` uncalled, and relay teleports will fail when the CLI tries to fetch/sign the Verifier domain.

Use the dedicated upgrade script:

```bash
export VERIFIER_PROXY=0xVerifierProxy
export PRIVATE_KEY=0x...
# Optional overrides; defaults shown below
export EIP712_NAME=Verifier
export EIP712_VERSION=1

forge script script/upgrade/VerifierUpgrade.s.sol:UpgradeVerifier \
  --rpc-url <VERIFIER_RPC> \
  --broadcast -vvvv
```

This script deploys a fresh implementation and executes:

```solidity
upgradeToAndCall(newImpl, abi.encodeCall(Verifier.initializeV2, ("Verifier", "1")));
```

Audit-pre checklist (functions not exercised in the walkthrough)
---------------------------------------------------------------
This repo has ZKP-heavy paths that are hard to exercise without proof generation. The walkthrough above mainly covers
deployment + peer wiring + the “unwrap and bridge” happy path. If you want additional lightweight checks without proofs,
these are the main public/external entrypoints that were NOT covered:

- `Hub`
  - Not exercised: `broadcast(...)`, `quoteBroadcast(...)`, `updateToken(...)`, `activateEmergency()`, `deactivateEmergency()`
  - View-only you can call anytime: `getTokenInfos()`, `getTransferRootsAndIndices()`, `currentAggregationRoot()`, `aggSeq()`
- `Verifier`
  - Not exercised: `reserveHashChain()` (no proof needed, but writes), `proveTransferRoot(...)` (needs Nova proof),
    `relayTransferRoot(...)` (needs proved root), `teleport(...)` / `singleTeleport(...)` (need proofs),
    `setVerifiers(...)`, `activateEmergency()` / `deactivateEmergency()`
  - View-only you can call anytime: `quoteRelay(...)`, `isUpToDate()`, `latestAggSeq()`, `globalTransferRoots(...)`
- `zERC20`
  - Not exercised: OFT send/receive flows (e.g. `send(...)` / compose-based flows)
  - Exercised indirectly via LiquidityManager: `mint(...)` (wrap), `burn(...)` (unwrap)
  - Admin setters exercised by scripts: `setVerifier(...)`, `setMinter(...)`
  - Blocklist enforcement: `BLOCKLIST` immutable set via constructor; `_update()` calls `BLOCKLIST.isBlocked()` for both `from` and `to`
- `Blocklist`
  - Admin functions: `blockAddress(...)`, `unblockAddress(...)`, `blockAddresses(...)` (onlyOwner)
  - View: `isBlocked(...)`
- `Adaptor`
  - Not exercised: `lzCompose(...)` (the common path when zERC20 arrives via OFT + compose), `decodeBridgeRequest(...)`,
    `bridgeZerc20Self(...)`
  - Exercised: `quoteFee(...)`, `unwrapAndBridge(...)`, `withdraw(...)`, balance views
- `LiquidityManager`
  - Not exercised: `unwrap(...)`, `quoteWrapReward(...)`, `quoteUnwrapFee(...)`, `setFeeParams(...)`, `withdrawRewards(...)`
  - Exercised: `wrap(...)`

Registering the Token on the Hub
--------------------------------
After deploying the verifier and token, register the new token with the Hub owner account:
```bash
cast send $HUB_ADDRESS \
  "registerToken((uint64,uint32,address,address))" \
  "($REMOTE_CHAIN_ID,$REMOTE_EID,$VERIFIER_ADDRESS,$TOKEN_ADDRESS)" \
  --rpc-url $HUB_RPC \
  --private-key $PRIVATE_KEY
```
- `$REMOTE_CHAIN_ID` is the EVM `chainid` of the verifier chain
- `$REMOTE_EID` must match the verifier`s `hubEid`
- Run `cast call $HUB_ADDRESS "eidToPosition(uint32)" $REMOTE_EID --rpc-url $HUB_RPC` to confirm the registration succeeded

Configuring LayerZero Peers After Deployment
-------------------------------------------
After every hub/verifier pair has been deployed and registered, wire the LayerZero peers using the dedicated Foundry scripts in `script/SetPeers.s.sol`. The order matters:

1. **Hub chain:** run `SetHubPeers` once to map every remote verifier EID to its address and register the associated token if it has not been registered yet.
2. **Each verifier chain:** run `SetVerifierPeers` separately so the verifier points back to the hub.

> Shortcut: the repo ships with `./run_set_peers.py` (with a `./run-set-peers.sh` wrapper), which reads a tokens config file (per-entry `eid` required) and exports the required environment variables before running both scripts in order. Provide extra forge flags after `--` (for example `./run_set_peers.py --file ../config/tokens.zusdc.testnet.json -- --broadcast -vv`) and ensure `PRIVATE_KEY` is set in your shell.

```bash
# Step 1: run on the hub chain (all verifiers at once)
export HUB_ADDRESS=0xHubOnThisChain
export VERIFIER_ADDRESSES=0xVerifierA,0xVerifierB
export VERIFIER_EIDS=40231,40232
export TOKEN_ADDRESSES=0xTokenA,0xTokenB
export TOKEN_CHAIN_IDS=421614,11155420
forge script script/SetPeers.s.sol:SetHubPeers \
  --rpc-url $HUB_RPC \
  --broadcast \
  -vvvv

# Step 2: run once per verifier chain
export HUB_ADDRESS=0xHubOnThisChain
export HUB_EID=40245
export VERIFIER_ADDRESS=0xVerifierOnThisChain
forge script script/SetPeers.s.sol:SetVerifierPeers \
  --rpc-url $VERIFIER_RPC \
  --broadcast \
  -vvvv
```

The helper contracts convert the hub address into the required 32-byte format automatically. Keep the environment variables scoped to the current chain before each run so that the correct RPC URL and addresses are used.

`SetHubPeers` registers new EIDs and calls `updateToken` for existing ones, so you can re-run the script safely as deployments change. Ensure each comma-separated list (`VERIFIER_ADDRESSES`, `VERIFIER_EIDS`, `TOKEN_ADDRESSES`, `TOKEN_CHAIN_IDS`) uses the same ordering so the data lines up per verifier.

Configuring LayerZero DVN / ULN Config
--------------------------------------
Use `script/SetDvnConfig.s.sol` to set ULN confirmations + DVN lists per OApp/remote EID. The helper `run_set_dvn_config.py` reads a per-chain JSON file plus a tokens config file and derives all routes automatically (verifier<->hub + token<->token).

DVN config files are located in `contracts/config/dvn/`:
- Testnet: `contracts/config/dvn/testnet/dvn-config.*.testnet.json`
- Mainnet: `contracts/config/dvn/mainnet/dvn-config.mainnet.json`

```bash
# Run derived routes (defaults to --broadcast)
./run_set_dvn_config.py --config contracts/config/dvn/testnet/dvn-config.zusdc.testnet.json -- --broadcast -vv
```

The config file points at a tokens config file (via `tokens_file`, e.g., `../config/tokens.zusdc.testnet.json`) and supplies two policies per token chain:
`verifier_hub` and `token`. The runner applies `verifier_hub` to both directions between hub and each verifier, and applies `token` to every outgoing token->token route from that chain.

DVN names must match the lz-address-book registry (see `getAvailableDVNs()` in `LZAddressContext` for discovery).

Troubleshooting Tips
--------------------
- Add `--resume` when rerunning a script that previously failed due to gas or fee settings
- Ensure the deployer wallet holds enough native gas token on every network involved
- If LayerZero fee quoting fails, double-check the endpoint address and confirm that the delegate has been granted the required permissions on the endpoint
