# zERC20 Architecture Overview

This document explains how the zk-wormhole-enabled ERC-20 (zERC20) system is split across on-chain contracts, Rust/TypeScript services, Internet Computer (ICP) canisters, and shared tooling inside this repository.

## Motivation and Design Goals

- **Privacy-preserving transfers:** a sender burns zERC20 to a stealth address and a recipient later mints on the same or another chain without linkable on-chain metadata.
- **ERC-20 compatibility:** token accounting, allowances, and mint/burn semantics reuse standard ERC-20 hooks so existing infrastructure works unmodified.
- **Provable integrity:** every mint relies on zero-knowledge proofs carried by Nova folding schemes or Groth16 circuits so the verifier contract can reproduce the burn set.
- **Scalability:** Poseidon Merkle trees, SHA-256 hash-chain checkpoints, and Incrementally Verifiable Computation (IVC) let off-chain services batch thousands of transfers into one proof.
- **Cross-chain reach:** LayerZero-based hub/verifier contracts keep per-chain trees in sync and let a recipient redeem on any registered chain.
- **Stealth coordination layer:** vetKD-powered canisters on the ICP deliver encrypted burn payloads and invoices so wallets and CLIs can coordinate off-chain without revealing recipients.

## Component Map

### On-Chain Contracts (`contracts/`)

| Component | Role |
| --- | --- |
| `zERC20` (`src/zERC20.sol`) | Upgradeable ERC-20 that emits `IndexedTransfer` events, maintains the truncated SHA-256 hash chain, and exposes `teleport` for verifiers plus `mint`/`burn` hooks for the minter. |
| `Verifier` (`src/Verifier.sol`) | LayerZero OApp + UUPS proxy that (a) reserves hash-chain checkpoints, (b) verifies Nova/Groth16 proofs, (c) tracks `totalTeleported` per recipient, and (d) relays local roots to the hub. |
| `Hub` (`src/Hub.sol`) | Aggregates every verifier’s latest root into a Poseidon tree and broadcasts the global root/sequence back to all verifiers through LayerZero. |
| `Minter` (`src/Minter.sol`) | Custodial bridge that mints/burns zERC20 in exchange for native/ERC-20 liquidity so users can enter and exit the system. |

### Circuits and Generated Artifacts

| Component | Location | Role |
| --- | --- | --- |
| Nova & Groth16 circuits | `zkp/` | Rust circuits for root transitions, batch withdraw, and single withdraw, plus the `generate_circuit_artifacts` binary that materialises setup artifacts. |
| Prover artifacts | `nova_artifacts/` | Output from `cargo run --release --bin generate_circuit_artifacts`; consumed by the indexer, decider-prover, CLI, WASM bindings, and Solidity verifier generators. |
| WASM bindings | `wasm/` | Wrap Nova + Groth16 proving in `wasm-pack` friendly exports (`WithdrawNovaWasm`, `SingleWithdrawWasm`) for the browser UI. |

### Off-Chain Services & Jobs

| Component | Location | Role |
| --- | --- | --- |
| Tree indexer | `indexer/` | Actix HTTP server backed by Postgres. Runs three jobs—event sync, Merkle tree ingestion, and the root prover job that compiles Nova proofs, requests decider proofs, and submits `proveTransferRoot`. |
| Decider prover | `decider-prover/` | HTTP worker that loads Nova parameters, accepts `CircuitKind::{Root,WithdrawLocal,WithdrawGlobal}` jobs, and returns decider proofs ready for on-chain verifiers. |
| Cross-chain job | `crosschain-job/` | Long-running worker that calls `relayTransferRoot` on every verifier and `Hub.broadcast` for all configured LayerZero EIDs using the shared `config/tokens.json`. |
| Docker orchestration | `docker/` | Compose files and entrypoints that bundle Postgres, the decider-prover, tree-indexer, and cross-chain job for local or staged deployments. |

### Client SDKs and Applications

| Component | Location | Role |
| --- | --- | --- |
| Shared Rust SDK | `client-common/` | Contract bindings (via Alloy), HTTP clients for the indexer/decider, burn-address + invoice helpers, and token metadata loaders reused by CLIs and services. |
| HTTP type schemas | `api-types/` | Serde-compatible request/response structs shared between the indexer/decider servers and their clients (Rust + TypeScript). |
| CLI | `cli/` | Commands for public transfers, stealth burns, invoice issuance/redemption, and teleport proof submission. Talks to the indexer, decider-prover, Hub/Verifier contracts, and ICP stealth canisters. |
| Frontend | `frontend/` | React + Vite application that mirrors the CLI flows in the browser. Loads WASM provers, the TypeScript stealth client, and the same HTTP APIs to guide users through private sends/receives. |
| Shared token config | `config/tokens*.json` | Canonical JSON metadata (RPC URLs, verifier addresses, hub info) consumed by the CLI, indexer, and cross-chain job. |

### ICP Stealth Storage (`zstorage/`)

| Component | Role |
| --- | --- |
| Key manager canister (`backend/key_manager`) | VetKD-backed canister that derives Boneh-Franklin IBE secrets per EVM address, enforces nonce + TTL on requests, and exposes Candid methods for recipients to fetch encrypted view keys. |
| Storage canister (`backend/storage`) | Persists announcements (IBE ciphertext + AES-GCM payload) and signed invoices, serving paginated scans plus individual lookups. |
| Rust stealth client (`frontend/`) | `StealthCanisterClient` wrapper plus higher-level helpers for encryption/decryption, invoices, and PocketIC integration. Imported by the CLI. |
| TypeScript stealth client (`frontend/src/services/sdk/storage`) | Browser-friendly client that shares the same Candid bindings and vetKD logic so the UI can issue invoices, publish announcements, and scan for inbound burns. |
| Specs & guides | `docs/zstorage/*.md` | Deployment, environment, and protocol references for the ICP layer. |

## Core Cryptographic & Messaging Primitives

- **Hash-chain commitments:** every `_update` on zERC20 appends `hashChain = SHA256(hashChain || to || value)` and exposes the truncated 248-bit digest. Verifiers reserve checkpoints with `reserveHashChain` so Nova proofs can be tied to immutable public inputs.
- **Indexed transfer events:** `IndexedTransfer(index, from, to, value)` events guarantee deterministic ordering. Postgres rows mirror these indexes, enabling consistent Merkle indices across all services.
- **Poseidon Merkle trees:** the indexer maintains partitioned Poseidon trees per token plus historical snapshots; contracts and circuits share the same circom-compatible parameters.
- **Full burn addresses & PoW filter:** helpers in `client-common::payment` derive `FullBurnAddress` payloads from a `(chain_id, recipient, secret, tweak)` tuple and enforce a 12-bit proof-of-work window so collision attacks remain costly.
- **General recipient binding:** Poseidon hashes `(chain_id, address, tweak)` into `GeneralRecipient.fr`, ensuring withdrawals can only mint to the intended destination.
- **Incrementally Verifiable Computation:** Nova folding proofs summarize either root transitions (tree growth + hash-chain updates) or batch withdrawals (ordered burn leaves). Groth16 circuits cover the single-withdraw path.
- **VetKD stealth messaging:** the key manager canister derives an identity-based encryption (IBE) key per recipient, encrypts it to a transport public key, and senders publish AES-GCM payloads + IBE wrappers to the storage canister. Only the intended recipient can decrypt announcements or invoices.
- **LayerZero aggregation tree:** the Hub stores each verifier’s latest root as Poseidon leaves and broadcasts a global root + sequence so verifiers can admit cross-chain teleports.

## Proof & Teleport Pipeline

### Event ingestion and root proving

1. **On-chain emission:** every zERC20 mint/transfer/burn updates the hash chain and emits `IndexedTransfer`.
2. **Event sync job (`indexer/`):** pulls contiguous logs from each configured RPC endpoint and stores them in Postgres (one table per token).
3. **Tree ingestion job:** reads newly indexed events and appends `(to, value)` leaves into the Poseidon tree tables, retaining historical roots for proof queries.
4. **Root prover job:** for each token it:
   - Initializes Nova with the reserved base index from Postgres.
   - Streams fresh events + Merkle proofs from the DB tree and extends the IVC proof.
   - Persists serialized IVC snapshots while waiting for the on-chain index to catch up.
   - Calls the decider-prover (`CircuitKind::Root`) to obtain a verification-ready proof and `reserve_hash_chain` / `prove_transfer_root` on the verifier once the hash chain checkpoint matches.
5. **Contract state:** successful submissions advance `latestProvedIndex`, emit `TransferRootProved`, and leave new roots ready for teleport proofs.

### Withdrawal / teleport (local & global)

1. **Stealth burn:** wallets or the CLI derive one or more `FullBurnAddress` payloads (optionally linked to invoices) and send zERC20 to those addresses. Senders typically publish the encrypted payload to `zstorage` so recipients can discover it.
2. **Recipient discovery:** CLI/Frontend users scan the storage canister, request encrypted view keys from the key manager via vetKD, and recover the `FullBurnAddress` payloads plus secrets/tweaks.
3. **Proof inputs:** clients call the indexer HTTP API to fetch eligible events, Merkle paths, and tree indices (local roots or global aggregation roots). `client-common` handles event bucketing and `general_recipient` derivations.
4. **Nova / Groth16 proving:** 
   - Batch withdrawals use Nova params from `nova_artifacts/` (or browser WASM) to fold all leaves plus dummy padding, then send the serialized IVC proof to the decider-prover for finalization (`CircuitKind::WithdrawLocal` or `WithdrawGlobal`).
   - Single withdrawals load Groth16 keys from the same artifacts and produce a calldata blob locally (no decider step).
5. **Verifier submission:** the CLI/Frontend composes `teleport` (Nova) or `singleTeleport` (Groth16) transactions that include the decider proof, recipient binding, leaf indices, and optional aggregation sequence. The contract mints the delta if the requested total exceeds `totalTeleported[recipientHash]`.

### Cross-chain aggregation upkeep

1. **`relayTransferRoot`:** verifiers periodically push their latest proved root + tree index to the Hub through LayerZero. The cross-chain job ensures this happens even if no proof submissions are pending.
2. **Hub broadcast:** when all leaves are fresh (or `broadcast` is manually triggered) the Hub Poseidon-hashes the leaves into a height-6 tree, increments `aggSeq`, and sends `(globalRoot, aggSeq)` payloads to every registered verifier.
3. **Global teleports:** recipients referencing `isGlobal=true` proofs must cite an aggregation sequence that the destination verifier already ingested. Local teleports skip the Hub and only require a per-token root.

## Stealth Announcements & Invoice Flows

1. **Invoice issuance:** wallets/CLI call `storage.submit_invoice` with a signed payload produced by `storage::invoice_signature_message`. Each invoice deterministically maps to either a single burn address or a batch of addresses derived from the signer’s seed.
2. **Announcement publishing:** senders fetch the recipient’s IBE public key from the key manager, derive a random AES key, encrypt the `FullBurnAddress` (or invoice reference), wrap the AES key in an IBE ciphertext, and submit the tuple to the storage canister.
3. **Recipient scanning:** users periodically run `scan_receive_transfers` (CLI) or the browser scanner. Both rely on the stealth clients to:
   - Request encrypted view keys (authenticated by EVM signatures + transport keys).
   - Decrypt announcements page-by-page and persist matches locally (`cli/output.json`) or in IndexedDB for the frontend.
4. **Redemption:** decrypted payloads feed into the teleport pipeline above, letting recipients redeem via CLI, browser UI, or offline scripts.

## Data Flow Summary

```text
1. contracts/zERC20::_update → emits IndexedTransfer + updates hashChain
2. indexer (event job) → stores events in Postgres; tree job appends leaves & roots
3. indexer (root job) → compiles Nova IVC, requests decider proof, calls reserveHashChain + proveTransferRoot
4. crosschain-job → relays roots to Hub and triggers Hub.broadcast for new aggregation roots
5. Hub.broadcast → Poseidon-aggregates leaves, pushes (globalRoot, aggSeq) to verifiers
6. zstorage (ICP) → distributes encrypted FullBurnAddress/invoice payloads; recipients obtain secrets via vetKD
7. CLI / frontend → fetch events + proofs from indexer, generate Nova/Groth16 withdraw proofs (local or global), request decider proofs when needed
8. contracts/Verifier.teleport → verifies proof, checks totals vs totalTeleported, instructs zERC20 to mint the delta
```
