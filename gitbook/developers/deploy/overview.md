# Deploy Your Own zERC20

This guide walks through deploying a custom zERC20 token (e.g., zUSDT) from scratch: contracts, infrastructure, and end-to-end testing.

## What You Will Deploy

A full zERC20 deployment consists of the following system components:

1. **Hub** (on a base chain, e.g., Base) --- aggregates transfer roots from all chains
2. **Verifier + zERC20** (per chain) --- ZKP verification + ERC-20 token
3. **LiquidityManager + Adaptor** (per chain) --- wrap/unwrap + cross-chain bridge via Stargate
4. **Indexer** --- syncs on-chain events, builds Merkle trees, generates IVC proofs
5. **Decider Prover** --- converts Nova IVC proofs to Groth16 proofs
6. **Crosschain Job** --- relays transfer roots between chains via LayerZero
7. **Fee Manager** (optional) --- dynamically adjusts wrap/unwrap fee parameters

## Prerequisites

- Foundry toolchain (`forge`, `cast`, `anvil`)
- Docker and Docker Compose
- Rust toolchain (stable + nightly for decider)
- Node.js >= 18
- Funded deployer wallets on each target chain
- RPC endpoints for each chain (e.g., Alchemy)
- LayerZero endpoint IDs (EIDs) for target chains

## Deployment Checklist

1. [Deploy Hub contract](contracts.md#deploying-the-hub) on base chain
2. [Deploy Verifier + Token](contracts.md#deploying-verifier-and-token) on each chain
3. [Deploy LiquidityManager + Adaptor](contracts.md#deploying-liquiditymanager-and-adaptor) on each chain
4. [Wire LayerZero peers](contracts.md#wiring-layerzero-peers) (Hub <-> Verifiers, Token <-> Token)
5. [Configure DVN](contracts.md#configuring-dvn)
6. [Register tokens on Hub](contracts.md#registering-tokens-on-hub)
7. [Create tokens.json config](contracts.md#creating-tokensjson)
8. [Download circuit artifacts](infrastructure.md#circuit-artifacts)
9. [Start infrastructure services](infrastructure.md#running-the-full-stack)
10. [Test end-to-end](end-to-end.md)

## Next Steps

- [Contract Deployment](contracts.md) --- detailed contract deployment steps
- [Infrastructure Setup](infrastructure.md) --- indexer, decider, crosschain job, fee manager
- [End-to-End Walkthrough](end-to-end.md) --- full testnet scenario
