# zERC20 Architecture Overview

This document explains the core ideas behind **zk-wormhole-enabled ERC-20 (zERC20)** and how the repository’s components fit together. The aim is to make the protocol approachable for engineers who are new to the project while still capturing the key architectural decisions.

## Motivation and Design Goals

- **Privacy-preserving transfers:** let a sender burn tokens on one chain and have a recipient mint them on the same or another chain without revealing the linkage.
- **ERC-20 compatibility:** use standard hooks so existing tooling such as wallets, explorers, and accounting systems continue to work.
- **Provable integrity:** rely on zero-knowledge proofs (ZKPs) so that mint operations are only accepted when the corresponding burns are proven.
- **Scalability:** accumulate many transfers into succinct proofs by combining hash-chain commitments, Poseidon Merkle trees, and Incrementally Verifiable Computation (IVC).
- **Cross-chain reach:** extend the mechanism from a single token contract to a hub that synchronises multiple verifier instances across chains.

## High-Level Components

| Component                       | Location          | Role                                                                                                                                                                                         |
| ------------------------------- | ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `zERC20` token contract       | `contracts/`      | Standard ERC-20 with (1) sequential hash-chain commitments and (2) indexed transfer events emitted for every mint, transfer, or burn.                                                        |
| Verifier contract               | `contracts/`      | Stores the latest proven Merkle roots, relays them to the hub via LayerZero, tracks how much each recipient has already teleported, and acts as the single on-chain entry point for minting. |
| Hub contract                    | `contracts/`      | Maintains a binary aggregation tree of transfer roots received from verifiers on multiple chains and broadcasts the aggregated root back to them.                                            |
| Nova circuits + folding schemes | `zkp/`            | Encode the ZK logic that rebuilds the transfer Merkle tree, enforces correct sequencing, and checks stealth recipient bindings.                                                              |
| Decider prover service          | `decider-prover/` | Runs the onchain proof generation using the Nova artifacts. Exposed over HTTP for the CLI.                                                                                                 |
| Indexer service                 | `indexer/`        | Listens to the token contract events, materialises the transfer Merkle tree off-chain, and supports range queries used during teleport.                                                      |
| CLI                             | `cli/`            | Wraps the prover and indexer APIs, derives stealth addresses from a private key, and submits teleport/transfer transactions.                                                                 |
| Shared client crate             | `client-common/`  | Provides strongly typed bindings to contracts and common data structures such as token metadata.                                                                                             |

## Core Cryptographic Primitives

- **Hash chain commitment:** every `_update` (mint, transfer, burn) updates `hashChain = H(previous, to, value)`. The on-chain value lets off-chain provers anchor the event order.
- **Indexed transfer events:** each event carries a monotonically increasing `index`, the recipient, and the raw amount.
- **Poseidon Merkle tree:** the indexer rebuilds transfers into a fixed-depth Poseidon tree; each leaf is `(to, value)`. Proven roots are fed into recursive circuits.
- **General recipient helper:** a Poseidon-based hash binds `(chain_id, address, tweak)` and produces the stealth burn address as a unique function of the intended recipient.
- **Incrementally Verifiable Computation:** Nova circuits fold many transfer checks into a single succinct proof. The proof records both the new hash chain head and the new Merkle root to guarantee they evolve consistently.

## Teleport Lifecycle

### Local teleport (single chain)

1. **Burn:** sender transfers funds to a stealth address derived from the recipient helper; on-chain this looks like a transfer to an otherwise unused address.
2. **Indexing:** the indexer records the transfer leaf and exposes Merkle inclusion proofs for its index range.
3. **Proof generation:** the decider prover folds the relevant transfers, ensuring each leaf’s destination matches the recipient helper and accumulates the total value.
4. **Submission:** the CLI submits the ZK proof to the verifier along with the recipient helper data.
5. **Mint:** if the proof is valid and the total exceeds the `totalTeleported` watermark, the verifier mints the delta to the recipient.

### Global teleport (multi-chain)

1. Steps 1–3 mirror the local flow but may involve transfers from different chains that share the same recipient helper.
2. Each verifier relays its latest transfer root to the hub via LayerZero. The hub stores the leaf in an aggregation tree and broadcasts a new `aggregationRoot` plus sequence number.
3. Once the target chain’s verifier sees the matching aggregation root, it accepts global teleport proofs that reference that sequence.
4. The verifier mints the delta and updates both `totalTeleported` and the latest aggregation sequence for the recipient.

## Data Flow Summary

```text
1. zERC20::_update → hashChain, IndexedTransfer(index, to, value)
2. Indexer → Poseidon Merkle tree (transferRoot)
3. Nova prover → recursive proof (old/new root, hash chain head, recipient binding, value sum)
4. Verifier → validates proof, stores transferRoot, relays to hub
5. Hub → aggregates cross-chain roots, broadcasts aggregationRoot
6. Verifier.teleport → checks aggregationRoot (global) or local root, mints delta
```

## Repository Structure Cheat Sheet

- `zkp/`: Rust circuits, gadgets, and utilities used to build Nova-based proofs.
- `cli/`: Binary crate (`cargo run`) offering `generate`, `transfer`, and `teleport` commands; consumes `client-common` bindings.
- `client-common/`: Shared contract abstractions, token metadata handling, and RPC providers.
- `contracts/`: Solidity sources for the token, verifier, hub, and auxiliary libraries.
- `decider-prover/`: Service wrapper that keeps the Nova folding state and serves proof requests.
- `indexer/`: Event ingestion pipeline plus HTTP API for fetching indexed transfers.
- `nova_artifacts/`: Generated proving parameters that both the decider and CLI expect (keep up to date when circuits change).
- `docs/`: Design notes (this file, plus the original Japanese drafts).

## Operational Considerations

- **RPC selection:** each token entry must specify at least one RPC endpoint so the CLI and services can query balances and submit transactions.
- **Artifacts freshness:** regenerate `nova_artifacts/` and rebuild the decider when the circuits change; the CLI can point to a custom path via `--nova-artifacts-dir`.
- **Service trust:** verifiers trust the hub to distribute the correct aggregation root, but emergency paths exist if conflicting roots are observed for the same index.
- **Testing:** run `cargo test --workspace` for circuits and shared logic, `cargo fmt` before commits, and use `wasm-pack build zkp --target web` when exercising WebAssembly proof generation.
