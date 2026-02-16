# Technical Specifications

Detailed technical specifications for zERC20 components.

## Specifications

- [Contract Spec](contract-spec.md) — Smart contract interfaces and behaviors
- [ZKP Spec](zkp-spec.md) — Zero-knowledge circuit specifications
- [ICP Storage Spec](icp-storage-spec.md) — Stealth messaging layer on Internet Computer

## Quick Reference

### Value Constraints

| Constraint | Value | Reason |
|------------|-------|--------|
| Max transfer value | 2^248 - 1 | Must fit in BN254 scalar field |
| Address size | 160 bits | Ethereum address width |
| Merkle tree depth | 40 (local) / 46 (global) | Per-token and global trees |
| PoW difficulty | 16 bits | Burn address collision resistance |

### Proof Types

| Proof Type | Circuit | Use Case |
|------------|---------|----------|
| Root Transition | Nova | Proving new transfer roots |
| Batch Withdraw | Nova | Multiple withdrawals in one proof |
| Single Withdraw | Groth16 | Single withdrawal (faster) |

### Key Contracts

| Contract | Main Functions |
|----------|---------------|
| zERC20 | `transfer`, `teleport`, `mint`, `burn` |
| Verifier | `proveTransferRoot`, `teleport`, `singleTeleport` |
| Hub | `broadcast`, `registerToken` |
