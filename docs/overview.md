## Preface

This note describes how to embed the **zk-wormhole** mechanism directly into ERC-20 so that the resulting token, **zk-wormhole-enabled ERC-20 (zERC20)**, simultaneously preserves privacy, verifiability, and compatibility.

The core idea proposed in **EIP-7503: Zero-Knowledge Wormholes / Private Proof of Burn (PPoB)** in 2023 can be summarised as follows:

> **Tokens sent to a non-existent (burn) address can be treated as irreversibly burned.**  
> By reproducing that burn inside a ZK proof and minting the same amount at a different address,  
> funds can be teleported without revealing the relationship between sender and recipient.

zERC20 develops this concept further. By combining strict ERC-20 compliance with hash-chain commitments, Poseidon Merkle reconstruction, and Incrementally Verifiable Computation (IVC), the construction becomes practical and scalable for production deployments.

---

## Key Contributions

### 1. Hash-chain commitments for transfers and IVC Poseidon Merkle reconstruction

Every transfer (including mints and burns) is appended to an on-chain hash chain.  
Off-chain, the sequence is reconstructed as an ordered Poseidon Merkle tree.  
On-chain state changes are reduced to a single `bytes32` update, while off-chain proofs can be produced incrementally and cheaply thanks to IVC.

### 2. Recipient-bound burn addresses

The burn address is derived from the intended recipient, so double-withdrawal resistance does not rely on per-transfer nullifiers.  
Instead, the contract tracks the **recipient’s cumulative withdrawals (`totalTeleported`)**.

```text
burnTo = poseidon(TRANSFER_DOMAIN_TAG, hiddenSalt, recipientChainId, recipientAddress)
```

Because multiple transfers can be verified within one ZK proof, the design supports batched withdrawals.

### 3. Cross-chain zk-wormhole extension

By embedding `recipientChainId` in the burn address,  
the protocol supports **burning on Chain A followed by private minting on Chain B**.  
A global aggregation tree keeps the hash commitments consistent across chains, enabling private, cross-chain teleports.

## Overview

### Design highlights

`zERC20` adds two battletested structures to the standard ERC-20 interface:

1. **Hash-chain commitment**  
   Each transfer (including mints and burns) is sequentially committed using SHA-256 or Keccak-256.
2. **Indexed transfer events**  
   A monotonically increasing `index` preserves ordering so that the Poseidon Merkle tree can be reconstructed without ambiguity.

With these additions, the ZK circuit can deterministically recover the transfer order, and IVC can generate proofs incrementally.

---

## State variables and events

```solidity
uint256 public index;       // Sequential identifier for IndexedTransfer
bytes32 public hashChain;   // Rolling hash of all transfers

event IndexedTransfer(uint256 indexed index, address indexed to, uint256 value);
event Teleport(address indexed to, uint256 value);
```

---

## ERC-20 update hook

The OpenZeppelin `_update` hook is extended so that every transfer, mint, and burn updates the hash chain.

```solidity
function _update(address from, address to, uint256 value) internal override {
    hashChain = computeHashChain(hashChain, from, to, value);
    super._update(from, to, value);
    emit IndexedTransfer(index++, to, value);
}
```

Including both `from` and `to` in `computeHashChain` prevents ambiguity between burns and mints that share the same recipient and amount.

The hash-chain update follows this shape:

```
H = Hf(prevHash || from || to || value || DOMAIN_SEPARATOR)
```

- `Hf` is either SHA-256 or Keccak-256, depending on deployment requirements.
- `DOMAIN_SEPARATOR` ensures compatibility with future extensions such as cross-chain tagging.

## Teleport function (verifier only)

```solidity
function teleport(address to, uint256 value) external onlyVerifier {
    _mint(to, value);
    emit Teleport(to, value);
}
```

After the verifier validates an off-chain ZK proof, it mints only the delta (`delta`) into the recipient account (see below for the verifier’s responsibilities).

---

## Transfer Merkle tree

Every `IndexedTransfer` becomes a leaf in the Poseidon Merkle tree.

```
Leaf(i) = { to: address, value: uint256 }
```

- Empty nodes use `ZERO_LEAF = Hf(0)`.
- The tree height is fixed (`TRANSFER_TREE_HEIGHT`).
- IVC constrains the correspondence between `hashChain` and the resulting `transferMerkleRoot`.

### Circuit constraints

- Initial state:  
  `prev_transfer_count = initial_transfer_count`  
  `prev_hash_chain = initial_hash_chain`  
  `prev_transfer_root = initial_transfer_root`  
  `new_transfer_count = initial_transfer_count`  
  `new_hash_chain = initial_hash_chain`  
  `new_transfer_root = initial_transfer_root`
- Each step:
  1. Recompute the hash chain: `new_hash_chain == computeHashChain(prev_hash_chain, transfer)`
  2. Update the Merkle root using the provided sibling path.
  3. Enforce `new_transfer_count = prev_transfer_count + 1`.

---

## Stealth transfers (burn side)

Recipients create stealth addresses using a secret `hiddenSalt`.

```
stealthTo = trim20(
  poseidon(TRANSFER_DOMAIN_TAG, hiddenSalt, recipientChainId, recipientAddress)
)
```

Because collisions with EOAs or contract addresses are computationally infeasible, sending to `stealthTo` can be regarded as a burn.

## Teleportation proof (mint side)

### Circuit constraints

- Initial witness: `last_transfer_index = 0`, `total_value = 0`
- For each step:
  1. Verify Merkle inclusion (sibling length is fixed).
  2. Check the derived recipient:  
     `transfer.to == trim20(poseidon(TRANSFER_DOMAIN_TAG, hidden_salt, recipient_chain_id, recipient_address))`
  3. Ensure monotonic indices: `(transfer.index + OFFSET) > last_transfer_index` with `OFFSET = 1`.
  4. Accumulate value: `total_value += transfer.value`.
  5. Update the marker: `last_transfer_index = transfer.index + OFFSET`.

### On-chain verification

```solidity
require(recipientChainId == block.chainid);
require(transferRoot == localRoot);
require(totalValue >= totalTeleported[recipientAddress]);

uint256 delta = totalValue - totalTeleported[recipientAddress];
if (delta > 0) {
    teleport(recipientAddress, delta);
    totalTeleported[recipientAddress] = totalValue;
}
```

---

## Cross-chain extension (zk-wormhole)

Each chain submits its `transferMerkleRoot` to a root hub.  
The hub aggregates those roots into an **aggregation tree** and broadcasts the resulting `aggregationRoot` back to all chains.  
Every chain can then validate cross-chain teleports against the shared commitment and mint privately on the destination chain.

## References

- **EIP-7503: Zero-Knowledge Wormholes / Private Proof of Burn (PPoB)**, Ethereum Magicians (2023)  
  [https://eips.ethereum.org/EIPS/eip-7503](https://eips.ethereum.org/EIPS/eip-7503)  
  [https://ethereum-magicians.org/t/eip-7503-zero-knowledge-wormholes-private-proof-of-burn-ppob/15456](https://ethereum-magicians.org/t/eip-7503-zero-knowledge-wormholes-private-proof-of-burn-ppob/15456)
