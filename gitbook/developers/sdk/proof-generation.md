# Proof Generation

After scanning for incoming transfers (see [Receiving](receiving.md)), you generate a zero-knowledge proof to claim your tokens on-chain. The SDK provides a high-level `prepareRedeemTransaction()` function that handles proof mode selection, proof generation, and transaction assembly in a single call.

## Two Proof Modes

| Mode | Use case | Gas cost |
|------|----------|----------|
| **Single** (Groth16) | Claiming one transfer | Higher per-transfer |
| **Batch** (Nova + Decider) | Claiming multiple transfers at once | Lower per-transfer |

The SDK automatically selects the proof mode based on the number of eligible events in the `RedeemContext`.

## Recommended: prepareRedeemTransaction

The simplest way to generate proofs and build the on-chain transaction:

```typescript
import {
  prepareRedeemTransaction,
  submitRedeemTransaction,
} from "zerc20-client-sdk";

// redeemContext comes from collectRedeemContext() -- see Receiving page
const redeemTx = await prepareRedeemTransaction({
  redeemContext,
  burn,                         // BurnArtifacts
  teleportProofClient: sdk.teleportProofs,
  decider: sdk.decider,         // required for batch proofs
});

// Submit the prepared transaction
const { transactionHash } = await submitRedeemTransaction({
  writeProvider,    // EvmWriteProvider (e.g., viem WalletClient)
  tx: redeemTx,
  readProvider,     // optional: for receipt polling
});
```

`prepareRedeemTransaction` inspects the eligible events and chooses:
- **1 event** -- Groth16 single proof via `Verifier.singleTeleport()`
- **2+ events** -- Nova batch proof finalized through the Decider, submitted via `Verifier.teleport()`

## Single Proof (Advanced)

Use the low-level API when you need fine-grained control over the single proof workflow:

```typescript
const sdk = createSdk();

// `redeemContext` comes from collectRedeemContext() -- see Receiving page
const artifacts = await sdk.teleportProofs.createSingleTeleportProof({
  aggregationState: redeemContext.aggregationState,
  recipientFr,
  secretHex,
  event: redeemContext.events.eligible[0],
  proof: redeemContext.globalProofs[0],
});
```

## Batch Proof (Advanced)

Use the low-level API for batch proofs when you need to manage the Decider interaction yourself. Batch proofs are finalized through a **Decider** service that converts the Nova IVC proof into a Groth16 proof for on-chain verification.

```typescript
import { createSdk, HttpDeciderClient } from "zerc20-client-sdk";

const sdk = createSdk();
const decider = new HttpDeciderClient("https://decider.intmax.io");

const artifacts = await sdk.teleportProofs.createBatchTeleportProof({
  aggregationState: redeemContext.aggregationState,
  recipientFr,
  secretHex,
  events: redeemContext.events.eligible,
  proofs: redeemContext.globalProofs,
  decider,
});
```

If you already have a Decider proof from an external source, use `buildBatchRedeemTransaction()` to assemble the transaction without re-proving:

```typescript
import { buildBatchRedeemTransaction } from "zerc20-client-sdk";

const redeemTx = buildBatchRedeemTransaction({
  redeemContext,
  burn,
  deciderProof: proofBytes,  // Uint8Array from Decider
});
```

## Next Steps

- [Receiving](receiving.md) -- scanning for incoming transfers and building proof inputs
- [SDK Quick Start](quickstart.md) -- installation and first private send
- [Wrap and Unwrap](wrap-unwrap.md) -- converting between underlying tokens and zERC20
