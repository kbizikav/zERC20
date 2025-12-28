# Contract Specification

## Purpose And Trust Model

- The system provides a privacy-preserving wrapped asset (`zERC20`) that supports private proof-of-burn redemptions via zero-knowledge proofs. On each chain, a `Verifier` contract validates per-token transfer roots and teleport proofs, while a single cross-chain `Hub` contract aggregates all transfer roots to derive global state. Liquidity is anchored and released through `LiquidityManager`, with `Adaptor` handling cross-chain exit intent and recovery.
- Trusted actors are limited to (a) the deployer who sets immutable parameters and initial deciders/verifiers, and (b) the upgrade/owner roles on each upgradeable contract. Owner compromises enable reconfiguration of verifiers, token registries, liquidity policy, or bridge parameters.

## Components

### zERC20 (`contracts/src/zERC20.sol`)

- Upgradeable ERC-20 that overrides `_afterTokenTransfer` to (1) enforce `value <= 2^248 - 1`, (2) append `(to, value)` to a SHA-256 hash chain truncated to 248 bits, and (3) emit `IndexedTransfer(index++, from, to, value)` for deterministic ordering. This history feeds the proof system (`hashChain` and `index` are public inputs).
- Maintains `verifier` (allowed to call `teleport`) and a liquidity authority (allowed to mint/burn for liquidity entry and exit). Owner-only setters guard against zero addresses, except that the liquidity authority may be set to `address(0)` on chains that deliberately disable liquidity flows.
- `teleport(address to, uint256 value)` is invoked solely by the Verifier once a teleport proof succeeds, minting directly to the provided address. zERC20 keeps emitting the legacy `Teleport(to, value)` for backwards compatibility, while the Verifier’s `Teleport` event additionally records `{isGlobal, rootHint, transferRoot, GeneralRecipient}` for forensic linking.
- Exposes auxiliary mint/burn entrypoints for the liquidity authority (`mint`, `burn`) plus a UUPS upgrade hook restricted to `owner`.

### Verifier (`contracts/src/Verifier.sol`)

- Upgradeable OApp (LayerZero) + Pausable contract responsible for:
  1. Recording checkpoints of the zERC20 hash chain (`reserveHashChain`) so that Nova proofs can reference stable public inputs.
  2. Verifying Nova proofs for transfer-root transitions (`proveTransferRoot`) via `IRootDecider.verifyOpaqueNovaProof`. Mismatched roots for the same index trigger `EmergencyTriggered` and pause the contract until `deactivateEmergency` is called after rotating verifiers.
  3. Verifying Nova (`teleport`) or Groth16 (`singleTeleport`) withdrawal proofs for a `GeneralRecipient`. Both flows validate the claimed root (`provedTransferRoots` vs `globalTransferRoots`), enforce recipient binding (`gr.hash()` packs chain id, address, tweak plus a version byte), and ensure the teleported amount is strictly increasing in `totalTeleported[recipientHash]`. Only the delta is minted on zERC20 to prevent double-spends.
  4. Relaying the latest proved root to the Hub via LayerZero (`relayTransferRoot`) and ingesting aggregated roots from the Hub in `_lzReceive`, updating `globalTransferRoots` and advancing `latestAggSeq`.
- Teleport privacy trade-offs: `isGlobal=false` uses local per-token roots (faster, chain-scoped privacy), while `isGlobal=true` references Hub-derived global roots (requires Hub liveness, improves unlinkability).
- Admin functions: `setVerifiers` rotates the decider/verifier addresses atomically; `deactivateEmergency` resumes operation. Owner also controls UUPS upgrades.

### Hub (`contracts/src/Hub.sol`)

- Central LayerZero OApp that tracks each token’s latest transfer root (`transferRoots`) and monotonically increasing tree index (`transferTreeIndices`). Registration is owner-gated:
  - `registerToken` inserts a new token with `(chainId, eid, verifier, token)` metadata, allocating a leaf slot (capacity capped by `POSEIDON_MAX_LEAVES = 2^6 = 64`).
  - `updateToken` refreshes metadata without altering ordering (leaf index equals registration order).
- LayerZero receive path (`_lzReceive`) accepts `(transferRoot, transferTreeIndex)` payloads from verifiers. Updates occur only when the incoming index is newer so snapshots remain monotonic between broadcasts.
- `broadcast` snapshots the current leaves, computes a PoseidonT3 aggregation tree (height 6, zero nodes pre-computed in storage), increments `aggSeq`, and multicasts the `(globalRoot, aggSeq)` payload to the requested target EIDs. Any excess `msg.value` is refunded. The emitted `AggregationRootUpdated` event exposes both the leaf snapshot and their tree indices for auditing.
- Fee handling: `quoteBroadcast` estimates native fees; `broadcast` verifies sufficient funding and reverts if LayerZero attempts to charge LZ tokens (`LayerZeroTokenFeeUnsupported`). Each `_lzSend` uses identical payload/options.

### LiquidityManager (`contracts/src/liquidity/LiquidityManager.sol`)

- Custody and policy boundary for liquidity. It exists to keep zERC20 supply anchored to real underlying liquidity while expressing the incentive curve that decides when liquidity should be attracted or released.
- Acts as the sole liquidity authority for minting/burning zERC20 on that chain, ensuring supply changes are traceable to explicit liquidity entry/exit policy.
- Accumulates fee surplus as a governance-controlled reserve that funds future incentives or withdrawals.

### Adaptor (`contracts/src/liquidity/Adaptor.sol`)

- Cross-chain exit and recovery boundary. It exists to honor user intent when converting zERC20 into underlying liquidity across chains, while providing a safe fallback when bridge conditions or fees make the exit untenable.
- Coordinates with the LiquidityManager for unwrapping and uses Stargate as the transport, while ensuring surplus fees and refunds remain attributable to the initiating user.

## Key Flows

### Private Proof Of Burn Lifecycle

1. **Transfer commitment**: Every zERC20 transfer emits `IndexedTransfer` and updates the truncated SHA-256 `hashChain`. Both `index` and `hashChain` are read by `Verifier.reserveHashChain`, which snapshots them into `reservedHashChains[index]`.
2. **Transfer root proving**: Indexers execute the Nova circuit that transitions from `oldRoot` to `newRoot` using the reserved checkpoint as a public input. Upon submitting `proveTransferRoot`, the contract verifies the proof, enforces consistency with `reservedHashChains`, and records `provedTransferRoots[newIndex]`. Divergent proofs for the same `newIndex` pause the verifier.
3. **Teleport (local or global)**: Users compile either a Nova (`teleport`) or Groth16 (`singleTeleport`) proof showing cumulative transfers to burn addresses represented by `GeneralRecipient`. The verifier cross-checks the claimed root (`rootHint` selects either local or global arrays), confirms the recipient hash and chain id match the caller’s environment, ensures the requested total exceeds the previously teleported amount, and mints the delta on zERC20 via `IzERC20.teleport` while emitting `Verifier.Teleport` with `{isGlobal, rootHint, transferRoot, GeneralRecipient}` so every mint is traceable to its origin.
4. **Global aggregation**: Verifiers periodically call `relayTransferRoot` so the Hub ingests `(root, index)` through LayerZero. Once multiple verifiers have contributed, the Hub calls `broadcast`, which Poseidon-aggregates the per-token roots, increments `aggSeq`, and sends the new global root back to every verifier. These global roots enable cross-chain teleports (`isGlobal=true`) without waiting for remote relays.

### Liquidity Entry / Exit Flow

1. Users enter the system by wrapping underlying through `LiquidityManager`, which exists to anchor zERC20 supply to available liquidity while applying incentive policy around the target liquidity band.
2. Users exit either locally by unwrapping via `LiquidityManager` or cross-chain through `Adaptor`, which exists to preserve exit intent (slippage limits, refunds) when bridging the underlying to another chain.

## Security & Operational Notes

- **Value range**: Both zERC20 transfer values and hash-chain limbs are limited to 248 bits to stay within the BN254 scalar field. Violations revert (`ValueTooLarge`) before any state change.
- **Emergency handling**: Any Nova proof inconsistency pauses the Verifier, preventing further teleports or relays until the owner rotates deciders/verifiers and calls `deactivateEmergency`.
- **LayerZero hygiene**: Verifier and Hub both inherit `OAppUpgradeable`, restricting message acceptance to known endpoints (`hubEid` on Verifier, registered EIDs on Hub). Payload lengths and tree indices are validated before state mutation. During `initialize`, the `_delegate` argument MUST be the same address that will own the contract so that LayerZero callbacks and owner-only admin functions stay aligned.
