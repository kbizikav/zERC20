# Proof Generation

After scanning for incoming transfers (see [Receiving](receiving.md)), you generate a zero-knowledge proof to claim your tokens on-chain. The SDK handles proof generation internally -- you supply the data from `collectRedeemContext` and get back ready-to-submit calldata.

## Two Proof Modes

| Mode | Use case | Gas cost |
|------|----------|----------|
| **Single** (Groth16) | Claiming one transfer | Higher per-transfer |
| **Batch** (Nova + Decider) | Claiming multiple transfers at once | Lower per-transfer |

## Single Proof

Use this when redeeming a single eligible transfer.

```typescript
const sdk = createSdk();

// `redeemContext` comes from collectRedeemContext() -- see Receiving page
const proof =
  redeemContext.aggregationState.scope === "local"
    ? redeemContext.eligibleProofs[0]
    : redeemContext.globalProofs[0];

const artifacts = await sdk.proofs.createSingleTeleportProof({
  aggregationState: redeemContext.aggregationState,
  recipientFr,
  secretHex,
  event: redeemContext.events.eligible[0],
  proof,
});
```

Submit on-chain:

```typescript
import { getVerifierContract } from "zerc20-client-sdk";

const verifier = getVerifierContract({ publicClient, walletClient, address: verifierAddress });

const txHash = await verifier.write.singleTeleport([
  artifacts.proofCalldata,
  artifacts.publicInputs,
  artifacts.treeDepth,
]);
```

## Batch Proof

Use this when redeeming multiple eligible transfers at once. Batch proofs are finalized through a **Decider** service that converts the Nova IVC proof into a Groth16 proof for on-chain verification.

```typescript
import { createSdk, HttpDeciderClient } from "zerc20-client-sdk";

const sdk = createSdk();
const decider = new HttpDeciderClient("https://decider.intmax.io");
const proofs =
  redeemContext.aggregationState.scope === "local"
    ? redeemContext.eligibleProofs
    : redeemContext.globalProofs;

const artifacts = await sdk.proofs.runNovaProver({
  aggregationState: redeemContext.aggregationState,
  recipientFr,
  secretHex,
  events: redeemContext.events.eligible,
  proofs,
});

const deciderCircuit =
  redeemContext.aggregationState.scope === "local"
    ? "withdraw_local"
    : "withdraw_global";
const deciderProof = await decider.produceDeciderProof(
  deciderCircuit,
  artifacts.ivcProof,
);
```

Submit on-chain:

```typescript
const verifier = getVerifierContract({ publicClient, walletClient, address: verifierAddress });

const txHash = await verifier.write.teleport([
  deciderProof,
  artifacts.finalState,
  artifacts.steps,
]);
```

If you prefer a higher-level helper, `generateBatchTeleportProof(...)` in `operations/receive.ts` wraps the same `sdk.proofs.runNovaProver(...)` call and chooses the decider circuit from `aggregationState.scope` for you.

## Next Steps

- [Receiving](receiving.md) -- scanning for incoming transfers and building proof inputs
- [SDK Quick Start](quickstart.md) -- installation and first private send
- [Wrap and Unwrap](wrap-unwrap.md) -- converting between underlying tokens and zERC20
