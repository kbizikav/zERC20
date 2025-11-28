Contracts Deployment Guide
==========================

This document explains how to deploy the LayerZero Hub, verifier, and `zERC20` token contracts that live in this directory. All deployment flows rely on Foundry scripts located under `script/`.

Prerequisites
-------------
- Foundry toolchain (`forge`, `cast`, `anvil`) installed via `foundryup`
- RPC endpoints for each network you intend to deploy to (for example Base Sepolia, Arbitrum Sepolia, Optimism Sepolia)
- A funded deployer key with permission to manage LayerZero configuration for the selected networks
- Endpoint IDs (EIDs) and LayerZero endpoint contract addresses for every chain you plan to connect

Environment Variables
---------------------
The scripts consume environment variables through `vm.env*` helpers. Place the values in an `.env` file and load them with `source .env` before running any command.

### Shared
- `PRIVATE_KEY`: Hex-encoded private key for the broadcaster account (also used as the default delegate when overrides are omitted)
- `RPC_URL`: RPC endpoint that matches the target chain passed to `--rpc-url`

### Hub deployment (`DeployHub`)
- `HUB_EID` (uint32): LayerZero endpoint ID for the chain hosting the Hub (for reference/logging)
- `HUB_ENDPOINT` (address): LayerZero endpoint contract on that chain
- `HUB_DELEGATE` (address, optional): Account that will own the Hub and manage LayerZero config; defaults to the broadcaster wallet if omitted

### Verifier and token deployment (`DeployVerifierAndToken`)
- `TOKEN_NAME` (string): ERC20 token name
- `TOKEN_SYMBOL` (string): ERC20 token symbol
- `HUB_EID` (uint32): Hub endpoint identifier the verifier should target
- `VERIFIER_ENDPOINT` (address): LayerZero endpoint contract on the verifier chain
- `VERIFIER_DELEGATE` (address, optional): Account that can update verifier LayerZero config; defaults to the broadcaster wallet if omitted
- `ZERC20_MINTER` (address, optional): Default account granted the `MINTER_ROLE` on the token

### Sample `.env`
```bash
PRIVATE_KEY=0xabc123...
RPC_URL=https://base-sepolia.example

HUB_EID=40245
HUB_ENDPOINT=0x6EDCE65403992e310A62460808c4b910D972f10f
# HUB_DELEGATE=0xYourDelegate # optional; defaults to PRIVATE_KEY holder

TOKEN_NAME=zUSD
TOKEN_SYMBOL=zUSD
HUB_EID=40245
VERIFIER_ENDPOINT=0x6EDCE65403992e310A62460808c4b910D972f10f
# VERIFIER_DELEGATE=0xYourVerifierDelegate # optional; defaults to PRIVATE_KEY holder
# ZERC20_MINTER=0x0000000000000000000000000000000000000000

# Peer configuration scripts
# HUB_ADDRESS=0xHubOnThisChain
# VERIFIER_ADDRESSES=0xVerifierA,0xVerifierB
# VERIFIER_EIDS=40246,40247
# TOKEN_ADDRESSES=0xTokenA,0xTokenB
# TOKEN_CHAIN_IDS=84532,421614
# VERIFIER_ADDRESS=0xVerifierOnThisChain
```

Pre-deploy Checks
-----------------
```bash
forge build
forge test
```
Run these commands inside `contracts/` to ensure the workspace compiles and tests pass before broadcasting transactions.

Deploying the Hub
-----------------
```bash
forge script script/Deploy.s.sol:DeployHub \
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
forge script script/Deploy.s.sol:DeployVerifierAndToken \
  --rpc-url $VERIFIER_RPC \
  --broadcast \
  -vvvv
```
The minter value can be omitted (it defaults to the broadcaster when unset). The script logs the addresses of the token, verifier, and each deployed Nova decider contract and wires the verifier into the token automatically.

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

> Shortcut: the repo ships with `./run-set-peers.sh`, which reads `config/tokens.json` and `layerzero_deployments.json`, exports the required environment variables, and runs both scripts in order. Provide extra forge flags after `--` (for example `./run-set-peers.sh -- --broadcast -vv`) and ensure `PRIVATE_KEY` is set in your shell.

```bash
# Step 1: run on the hub chain (all verifiers at once)
export HUB_ADDRESS=0xHubOnThisChain
export VERIFIER_ADDRESSES=0xVerifierA,0xVerifierB
export VERIFIER_EIDS=40246,40247
export TOKEN_ADDRESSES=0xTokenA,0xTokenB
export TOKEN_CHAIN_IDS=84532,421614
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

Troubleshooting Tips
--------------------
- Add `--resume` when rerunning a script that previously failed due to gas or fee settings
- Ensure the deployer wallet holds enough native gas token on every network involved
- If LayerZero fee quoting fails, double-check the endpoint address and confirm that the delegate has been granted the required permissions on the endpoint
