# Proof of Innocence (Planned)

> **Status: Not yet implemented.** This page describes a planned feature. The interfaces and designs below are subject to change and are not available for use today.

Proof of Innocence is an off-chain compliance feature that acts as an AML policy. It allows users to prove their received funds did not originate from sanctioned addresses (e.g., OFAC list) without revealing transaction details.

## Overview

| Property | Description |
|----------|-------------|
| **Purpose** | Prove sender addresses are not on a sanctions list |
| **Proof System** | Groth16 (single proof) / Nova (aggregated) |
| **Verification** | Off-chain CLI verifier |
| **Data Structure** | Sparse Merkle Tree (SMT) for sanctions list |

## How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                    Proof of Innocence Flow                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. Prove that `totalTeleported` per recipient (hash of         │
│     `GeneralRecipient`) is not originated from OFAC-sanctioned  │
│     sources.                                                    │
│                        ↓                                        │            
│  2. Commit the OFAC list using a commitment scheme that         │
│     supports non-membership proofs (e.g., Exclusion Tree).      │
│                        ↓                                        │
│  3. For each `from_address`, generate non-membership proof.     │
│                        ↓                                        │
│  4. For each teleport, prove that `transfer_leaf.from` is not   │
│     in the OFAC list, then aggregate the steps with Nova.       │
│                        ↓                                        │
│  5. The Verifier checks the Nova proof with public inputs:      │
│     `recipient`, `totalTeleported`, and the trusted OFAC SMT    │
│     root.                                                       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Cryptographic Design

### Exclusion Tree

A “OFAC non-membership tree” is a Merkle tree of sorted disjoint (`start`, `end`) pairs representing the gaps between the elements of the OFAC sanctions list. That is, the union of the regions `start..=end` is exactly the complement of the set of sanctions list.
The OFAC sanctions list is committed using a OFAC non-membership tree. The leaf of the the tree are computed as:
```rust
leaf = poseidon2(start, end)
```
If the sanctions list contains **k** addresses, the exclusion tree has **k + 1** gaps (one before the first address, one between each consecutive pair, and one after the last).The minimum tree height is **⌈log₂(k + 1)⌉**. In practice, add 2-3 levels of headroom to accommodate list growth without regenerating circuit artifacts.

### Non-Membership Proof (Nova Step)
Each transfer to a recipient has a `transfer_leaf` = `(from_address, burn_address, value)` (where `burn_address` is the `to` field of the generic transfer leaf).
To prove that `totalTeleported` per `recipient` is not originated from sanctioned sources,
for each transfer we prove the sender (`transfer_leaf.from`) is not in the OFAC list:
**Public Inputs (constant across all steps):**
1. `ofac_root` - root of the OFAC non-membership tree
2. `recipient` - hash of GeneralRecipient
**Accumulator State:**
1. `totalTeleported` - running sum of transfer values
**Per-Step Private Inputs:**
1. `from_address` - sender of this transfer (`transfer_leaf.from`)
2. `value` - transfer amount
3. `start`, `end` - gap boundaries containing `from_address`
4. `path`, `position` - Merkle proof for the gap
**Per-Step Constraints:**
1. **Sender not sanctioned**: `start < from_address < end` verified against `ofac_root`
2. **Accumulate value**: `totalTeleported_new = totalTeleported_old + value`

## CLI Usage (Planned)

> **Note**: This is an MVP interface. Users must manually assemble witness files.

### Generate Proof of Innocence

Generate a Nova proof that all transfers to a recipient originated from non-sanctioned addresses.

```bash
# Generate proof for a recipient
zerc20-cli proof-of-innocence generate \
  --recipient <GENERAL_RECIPIENT_HASH> \
  --ofac-root <OFAC_TREE_ROOT> \
  --transfers-file <TRANSFERS_JSON> \
  --exclusion-proofs-file <EXCLUSION_PROOFS_JSON> \
  --output <PROOF_OUTPUT_PATH>
```

| Argument | Description |
|----------|-------------|
| `--recipient` | Hash of GeneralRecipient (from withdrawal flow) |
| `--ofac-root` | Trusted OFAC exclusion tree root |
| `--transfers-file` | JSON file with list of (from_address, value) pairs |
| `--exclusion-proofs-file` | JSON file with exclusion proofs from OFAC tree service |
| `--output` | Path to write the generated proof |

### Verify Proof of Innocence

```bash
# Verify a proof
zerc20-cli proof-of-innocence verify \
  --proof <PROOF_PATH> \
  --recipient <GENERAL_RECIPIENT_HASH> \
  --total-teleported <TOTAL_VALUE> \
  --ofac-root <OFAC_TREE_ROOT>
```

| Argument | Description |
|----------|-------------|
| `--proof` | Path to the proof file |
| `--recipient` | Expected recipient hash |
| `--total-teleported` | Expected total value (wei) |
| `--ofac-root` | Trusted OFAC exclusion tree root |

### Example

```bash
# 1. Generate proof
zerc20-cli proof-of-innocence generate \
  --recipient 0x1a2b3c...recipient_hash \
  --ofac-root 0x5678...ofac_root \
  --transfers-file ./my_transfers.json \
  --exclusion-proofs-file ./exclusion_proofs.json \
  --output ./innocence_proof.bin

# 2. Verify proof
zerc20-cli proof-of-innocence verify \
  --proof ./innocence_proof.bin \
  --recipient 0x1a2b3c...recipient_hash \
  --total-teleported 1000000000000000000 \
  --ofac-root 0x5678...ofac_root

# Output:
# ✓ Proof valid
# recipient: 0x1a2b3c...
# totalTeleported: 1000000000000000000 (1.0 ETH)
# ofac_root: 0x5678...
```

### Input File Formats

`transfers.json` - List of transfers to prove:

```json
[
  { "from_address": "0xabc...", "value": "500000000000000000" },
  { "from_address": "0xdef...", "value": "500000000000000000" }
]
```

`exclusion_proofs.json` - Exclusion proofs from OFAC tree service:

```json
[
  {
    "from_address": "0xabc...",
    "start": "0x000...",
    "end": "0x111...",
    "path": ["0x...", "0x..."],
    "position": 5
  },
  {
    "from_address": "0xdef...",
    "start": "0x222...",
    "end": "0x333...",
    "path": ["0x...", "0x..."],
    "position": 12
  }
]
```