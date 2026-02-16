# ZKP Specification

## Overview

zERC20 uses zero-knowledge proofs to verify private transfers without revealing the link between sender and recipient. All circuits operate over the BN254 scalar field.

**Location**: `zkp/src/circuits/`

## Constraints

| Constraint | Value | Reason |
|------------|-------|--------|
| Max value | 248 bits (31 bytes) | Must fit in BN254 scalar field |
| Address size | 160 bits | Ethereum address width |
| PoW difficulty | 16 bits | Burn address collision resistance |

## Circuits

### 1. Burn Address Derivation

Generates deterministic burn addresses with proof-of-work difficulty.

```
burn_address = truncate_160(Poseidon3(domain_separator, recipient, secret))

where:
  domain_separator = field_encode("burn")
  recipient = Poseidon3(chain_id, address, tweak)
```

**PoW Constraint**: Bits `[160, 160 + POW_DIFFICULTY)` must be zero, requiring ~2^16 trials to find a valid `(recipient, secret)` pair.

### 2. Single Withdraw (Groth16)

Proves a single transfer to a burn address exists in the Merkle tree.

**Public Inputs**:
- `merkle_root`: Current transfer tree root
- `recipient`: GeneralRecipient hash
- `withdraw_value`: Amount to withdraw

**Private Inputs**:
- `from_address`, `value`, `delta`
- `secret`, `leaf_index`, `siblings[]`

**Constraints**:
1. Recompute burn address from `(recipient, secret)` with PoW check
2. Hash leaf: `Poseidon3(from_address, burn_address, value)` — `burn_address` is the `to` field of the transfer leaf (see [Integration Guide](../integration.md#leaf-structure))
3. Verify Merkle inclusion against `merkle_root`
4. Output `withdraw_value = value - delta`

### 3. Withdraw Step (Nova)

Folding gadget for batch withdrawals. Aggregates multiple transfers while maintaining ordering.

**Accumulator State**:
- `merkle_root`: Transfer tree root
- `recipient`: GeneralRecipient (constant across batch)
- `leaf_index_with_offset`: Last processed index + 1
- `total_value`: Running sum of values

**Per-Step Inputs**:
- `from_address`, `value`, `secret`
- `leaf_index`, `siblings[]`
- `is_dummy`: Skip real verification (for padding)

**Constraints**:
1. Enforce ordering: `prev_leaf_index_with_offset < leaf_index + 1`
2. When `is_dummy = false`:
   - Verify burn address PoW
   - Verify Merkle inclusion
   - Add `value` to total
3. When `is_dummy = true`:
   - Skip PoW and Merkle checks
   - Subtract `value` from total (for privacy adjustment)

**Dummy Steps**: Allow batch length hiding and fractional remainder adjustments without touching the Merkle root.

### 4. Root Transition Step (Nova)

Links on-chain `IndexedTransfer` events with the off-chain Merkle tree.

**Accumulator State**:
- `index`: Current transfer index
- `hash_chain`: SHA-256 chain value (248 bits)
- `root`: Current Merkle root

**Per-Step Inputs**:
- `from_address`, `to_address`, `value`
- `siblings[]`
- `is_dummy`: Skip real verification

**Constraints**:
1. When `is_dummy = false`:
   - Verify previous root has zero leaf at `index`
   - Compute leaf: `Poseidon3(from_address, to_address, value)`
   - Update root with new leaf
   - Update hash chain: `SHA256(hash_chain || from_address || to_address || value)[0:248]`
   - Increment index
2. When `is_dummy = true`:
   - Pass through state unchanged

## Proof Types

| Type | Circuit | Prover | Use Case |
|------|---------|--------|----------|
| Single Withdraw | Groth16 | Local (WASM) | Single transfer, fast |
| Batch Withdraw | Nova + Decider | Server | Multiple transfers |
| Root Transition | Nova + Decider | Server | Proving new roots |

## Verification Path

### Single Teleport

```
Client → Groth16 proof → Verifier.singleTeleport() → singleWithdrawVerifier contract
```

### Batch Teleport

```
Client → Nova IVC proof → Decider Prover → Decider proof → Verifier.teleport() → withdrawDecider contract
```

### Root Proving

```
Indexer → Nova IVC proof → Decider Prover → Decider proof → Verifier.proveTransferRoot() → rootDecider contract
```

## Security Notes

- **PoW Security**: With 16-bit PoW, collision resistance is ~2^89 (see detailed analysis in source spec)
- **Padding**: Nova proofs should pad to maximum index to avoid leaking batch size
- **Hash Chain**: Truncated to 248 bits for BN254 compatibility
