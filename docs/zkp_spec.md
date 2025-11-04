# Zero-Knowledge Circuit Specification

This document captures the behavior and constraints enforced by the zk circuits found in `zkp/src/circuits`. The circuits operate over the BN254 scalar field and reuse shared gadgets for Poseidon hashing, Merkle trees, and SHA-256 based hash chaining.

## Shared Bounds and Utilities
* All token `amount` values are range-checked to fit in **31 bytes (248 bits)** so they remain strictly less than the BN254 scalar modulus and align with the Poseidon- and SHA-based gadgets (`BYTES31_BIT_LENGTH` in [`constants.rs`](../zkp/src/circuits/constants.rs)).
* Addresses are constrained to **160 bits** to match Ethereum-style account widths (`ADDRESS_BIT_LENGTH` in [`constants.rs`](../zkp/src/circuits/constants.rs)).
* The SHA-256 hash chain gadget truncates its output to the lower 248 bits so the digest can be embedded directly in the BN254 scalar field while staying compatible with the on-chain representation. 
## `burn_address_var`

This gadget generates the deterministic burn address that the withdrawal logic relies on while embedding a proof-of-work style difficulty check. It is used directly by both the single withdrawal circuit and the Nova folding step to bind withdrawals to a unique `(recipient, secret)` pair before the contract releases funds.

```
burn_address_var(
    poseidon_params,
    recipient,
    secret,
    is_constrained,
) -> burn_address
```

* Computes `poseidon_hash = Poseidon(recipient, secret)` using the Circom-compatible Poseidon gadget.
* When `is_constrained` is true, multiplies each bit in the range `[160, 160 + POW_DIFFICULTY)` by `is_constrained` and forces the product to zero, enforcing `POW_DIFFICULTY` leading zeros immediately above the address window.
* Truncates the hash to the lower 160 bits and returns the result as the burn address.
* The parameter `POW_DIFFICULTY` is currently 12, so the proof-of-work condition raises the collision cost from the ~`2^(160/2)` birthday-attack baseline to roughly `2^(160/2 + 12)`.
* Host helpers (`compute_burn_address_from_secret`, `find_pow_nonce`, `secret_from_nonce`) mirror the in-circuit behavior for witness generation.

## `single_withdraw`

This circuit underpins the direct withdrawal path exposed to the contract. It proves that a particular transfer leaf targeting the derived burn address exists in the off-chain transfer tree and releases the withdrawable value while masking exact amounts with a configurable delta.

```
single_withdraw(
    poseidon_params,
    merkle_root,
    recipient,
    value,
    delta,
    secret,
    leaf_index,
    siblings[DEPTH],
) -> withdraw_value
```

* Range-checks `leaf_index`, `value`, and `delta`; all must fit within the DEPTH-bit index window and the 31-byte amount bound.
* Recomputes the leaf address via `burn_address_var` with the PoW constraint enabled (`Boolean::constant(true)`), then hashes it with the `value` to obtain the leaf commitment.
* Rebuilds the Merkle root from the supplied siblings and enforces equality with the public `merkle_root` input, proving inclusion of the burn leaf.
* Outputs `withdraw_value = value - delta` and range-checks the result against the 31-byte limit.
* The `delta` offset allows the prover to shave off a configurable privacy buffer without altering the committed leaf value.

## `withdraw_step`

`withdraw_step` is the Nova folding gadget used to batch withdrawals. The recursive proof aggregates many transfers while maintaining ordering guarantees so the contract can accept a single proof for a whole batch while preventing double withdrawals.

```
withdraw_step(
    poseidon_params,
    merkle_root,
    recipient,
    prev_leaf_index_with_offset,
    prev_total_value,
    is_dummy,
    value,
    secret,
    leaf_index,
    siblings[DEPTH],
) -> (next_root, next_recipient, next_leaf_index_with_offset, next_total_value)
```

* Range-checks `leaf_index`, `prev_leaf_index_with_offset`, and `value`, then sets `leaf_index_with_offset = leaf_index + 1`.
* Enforces `prev_leaf_index_with_offset < leaf_index_with_offset` so the ordering constraint is compatible with the initial accumulator value of zero; the `+1` offset guarantees that an actual leaf at index `0` can still be processed without colliding with the starting state.
* Recomputes the burn address but skips the PoW constraint when `is_dummy` is true, allowing padded steps to bypass witness generation.
* When `is_dummy` is false the burn address must satisfy the same PoW window as in `single_withdraw`, so crafting a colliding withdrawal falls back to the ~`2^(160/2 + 12)` effort bound.
* Updates the Merkle root only when `is_dummy` is false, ensuring dummy padding never touches the authenticated tree.
* When `is_dummy` is true the circuit subtracts the provided `value` from the running total, letting the prover smooth out distinctive fractional remainders so that privacy is not degraded by uniquely sized withdrawals. Real leaves add their `value`, and every update is range-checked to 31 bytes.
* Returns the unchanged `merkle_root`, the passthrough `recipient`, the updated `leaf_index_with_offset`, and the new running total.
* Dummy steps maintain hiding of the actual batch length and allow balancing fractional adjustments without touching the Merkle root.

## `root_transition_step`

This Nova step circuit links the on-chain `IndexedTransfer` events with the off-chain transfer tree. Each recursion step proves that inserting a transfer into the tree produces the same root that the contract expects while simultaneously updating the SHA-256 hash chain tracked in the contract state. The `is_dummy` hook is also required because the Nova decider needs proofs spanning at least two steps; a single real transition can be paired with a dummy step so the decider proof remains valid.

```
root_transition_step(
    poseidon_params,
    index,
    prev_hash_chain,
    prev_root,
    address,
    value,
    siblings[DEPTH],
    is_dummy,
) -> (next_index, next_hash_chain, next_root)
```

* Range-checks the recipient `address` (160 bits) and transfer `value` (31 bytes). `index` is also limited to `DEPTH` bits via `to_bits_le_limited`.
* For real steps (`is_dummy == false`), verifies that the previous root corresponds to a zero leaf at `index` using the provided siblings. Dummy steps skip this check.
* Hashes the `(address, value)` pair into a leaf and recomputes the new Merkle root with the same path.
* Updates the SHA-256 hash chain by concatenating `prev_hash_chain || address || value`, taking the lower 248 bits of the digest, and conditionally applies the update when not dummy.
* Increments the transfer index for real steps (`index + 1`) and keeps it unchanged for dummy steps, mirroring the conditional updates applied to the hash chain and root.
* The zero-leaf precondition ensures each on-chain event corresponds to inserting a previously empty slot, keeping the tree consistent with contract semantics.
* Nova proofs that approach the tree capacity can always be started from an earlier, already-proven index so the recursion still runs for at least two steps without requiring the final `index = 2^DEPTH - 1` transition to carry over into a dummy step.

## Transfer Tree Height Parameterization

All Merkle operations take `DEPTH` as a const generic, so the same gadgets can enforce membership in per-token transfer trees (height `TRANSFER_TREE_HEIGHT`) and the global forest (`GLOBAL_TRANSFER_TREE_HEIGHT`). Callers must supply siblings whose length matches the chosen depth, and the range checks ensure `leaf_index` stays within that tree’s domain.

## Contract Verification Path

* `Verifier.proveTransferRoot` accepts Nova proofs generated from repeated applications of `root_transition_step`. The proof is relayed to the `rootDecider` contract, and upon acceptance the verifier checks that the old root matches its ledger, the reserved hash chain equals the proof output, and then stores the new transfer root.
* Batched withdrawals invoke `Verifier.teleport`, which decodes the Nova public inputs from the `withdraw_step` accumulator, ensures the initial index and total are zero, and asks either `withdrawGlobalDecider` or `withdrawLocalDecider` to validate the proof before releasing funds.
* For direct withdrawals `Verifier.singleTeleport` relies on Groth16 verifiers (`singleWithdrawGlobalVerifier`, `singleWithdrawLocalVerifier`). The proof must demonstrate a valid `single_withdraw` inclusion, after which the contract reuses the same root-matching and transfer logic as the batched path.
