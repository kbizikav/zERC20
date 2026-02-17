# Proof Generation

The SDK supports two proof modes for claiming private transfers:

1. **Single Groth16 proof** -- proves ownership of one burn address and produces calldata for a single teleport.
2. **Batch Nova + Decider proof** -- folds multiple transfers into a single Nova IVC proof, then finalizes it through the Decider service. More gas-efficient when claiming several transfers at once.

Both modes prove that the caller knows the secret behind one or more burn addresses and output ABI-encoded calldata ready for on-chain verification.

## WASM Setup

Zero-knowledge proof generation runs inside a WASM module. The `WasmRuntime` class manages the WASM lifecycle -- loading artifacts, initializing memory, and exposing the prover API to the rest of the SDK.

```typescript
class WasmRuntime {
  constructor(options?: WasmRuntimeOptions);
}
```

### WasmRuntimeOptions

| Field | Type | Description |
|-------|------|-------------|
| `locator?` | `ArtifactLocator` | Custom configuration for loading circuit artifacts (proving keys, R1CS, WASM binaries). Falls back to bundled defaults if omitted. |

The easiest way to obtain a `WasmRuntime` is through the top-level factory:

```typescript
import { createSdk } from "zerc20-client-sdk";

const sdk = createSdk();
// sdk.wasm is a ready-to-use WasmRuntime
```

You can also create a standalone instance:

```typescript
import { WasmRuntime } from "zerc20-client-sdk";

const wasm = new WasmRuntime();
```

> **Browser note:** In browser environments, WASM must be fully initialized before calling any proof function. The SDK handles this automatically when you use `createSdk()`.

## Single Proof

Generate a Groth16 proof for a single transfer.

```typescript
TeleportProofClient.createSingleTeleportProof(
  params: SingleTeleportParams,
): Promise<SingleTeleportArtifacts>;
```

### SingleTeleportParams

| Field | Type | Description |
|-------|------|-------------|
| `aggregationState` | `AggregationState` | Current on-chain Merkle aggregation state |
| `recipientFr` | `bigint` | Recipient address encoded as a BN254 scalar field element |
| `secretHex` | `string` | Hex-encoded secret used to derive the burn address |
| `event` | `IndexedEvent` | The indexed transfer event to prove (from the indexer) |
| `proof` | `GlobalTeleportProof` | Global Merkle inclusion proof for the transfer leaf |

### SingleTeleportArtifacts

| Field | Type | Description |
|-------|------|-------------|
| `proofCalldata` | `Hex` | ABI-encoded Groth16 proof, ready to pass to the Verifier contract |
| `publicInputs` | `bigint[]` | Public inputs for the proof (root, nullifier, recipient, etc.) |
| `treeDepth` | `number` | Depth of the Merkle tree used in the proof |

### Submit a Single Proof On-Chain

```typescript
import { getVerifierContract } from "zerc20-client-sdk";

const verifier = getVerifierContract({ publicClient, walletClient, address: verifierAddress });

const txHash = await verifier.write.singleTeleport([
  artifacts.proofCalldata,
  artifacts.publicInputs,
  artifacts.treeDepth,
  // ...additional contract arguments
]);
```

## Batch Proof

Fold multiple transfers into a single Nova IVC proof, then finalize it through the Decider prover service.

```typescript
TeleportProofClient.createBatchTeleportProof(
  params: BatchTeleportParams,
): Promise<BatchTeleportArtifacts>;
```

### BatchTeleportParams

Extends `NovaProverInput` with decider-related fields:

| Field | Type | Description |
|-------|------|-------------|
| `aggregationState` | `AggregationState` | Current on-chain Merkle aggregation state |
| `recipientFr` | `bigint` | Recipient address as a BN254 scalar field element |
| `secretHex` | `string` | Hex-encoded secret for burn-address derivation |
| `events` | `IndexedEvent[]` | Array of indexed transfer events to prove |
| `proofs` | `GlobalTeleportProof[]` | Corresponding global Merkle inclusion proofs |
| `decider` | `HttpDeciderClient` | Decider client instance used to finalize the IVC proof |
| `onDeciderRequestStart?` | `() => void` | Optional callback fired when the SDK begins polling the Decider service |

### BatchTeleportArtifacts

| Field | Type | Description |
|-------|------|-------------|
| `deciderProof` | `Uint8Array` | Finalized Decider proof bytes |
| `ivcProof` | `Uint8Array` | Raw Nova IVC proof (before Decider finalization) |
| `finalState` | `bigint[]` | Final accumulator state after folding all steps |
| `steps` | `number` | Number of Nova folding steps (equal to the number of transfers) |

### Submit a Batch Proof On-Chain

```typescript
import { getVerifierContract } from "zerc20-client-sdk";

const verifier = getVerifierContract({ publicClient, walletClient, address: verifierAddress });

const txHash = await verifier.write.teleport([
  artifacts.deciderProof,
  artifacts.finalState,
  artifacts.steps,
  // ...additional contract arguments
]);
```

## Decider Client

The Decider is an off-chain service that converts a Nova IVC proof into a Groth16 proof suitable for on-chain verification. The SDK communicates with it via HTTP.

```typescript
class HttpDeciderClient {
  constructor(baseUrl: string, options?: DeciderClientOptions);

  produceDeciderProof(
    circuit: CircuitKind,
    ivcProof: Uint8Array | string,
  ): Promise<Uint8Array>;
}
```

### DeciderClientOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `pollIntervalMs?` | `number` | `1000` | Milliseconds between status polls |
| `timeoutMs?` | `number` | `300000` | Maximum time (ms) to wait for the Decider to finish |
| `fetchImpl?` | `typeof fetch` | global `fetch` | Custom fetch implementation (useful for Node.js < 18 or testing) |

### CircuitKind

```typescript
type CircuitKind = "root" | "withdraw_local" | "withdraw_global";
```

| Value | Usage |
|-------|-------|
| `"root"` | Root-level aggregation circuit |
| `"withdraw_local"` | Local (same-chain) withdrawal proof |
| `"withdraw_global"` | Global (cross-chain) withdrawal proof |

### Usage

```typescript
import { HttpDeciderClient } from "zerc20-client-sdk";

const decider = new HttpDeciderClient("https://decider.intmax.io", {
  pollIntervalMs: 2000,
  timeoutMs: 600_000,
});

// Used internally by createBatchTeleportProof, or call directly:
const deciderProof = await decider.produceDeciderProof(
  "withdraw_global",
  ivcProofBytes,
);
```

## Full Example -- Single Proof End-to-End

```typescript
import {
  createSdk,
  getVerifierContract,
} from "zerc20-client-sdk";

const sdk = createSdk();

// Assume `event`, `proof`, and `aggregationState` are fetched from the indexer.
const artifacts = await sdk.teleportProofs.createSingleTeleportProof({
  aggregationState,
  recipientFr,
  secretHex,
  event,
  proof,
});

const verifier = getVerifierContract({
  publicClient,
  walletClient,
  address: verifierAddress,
});

const txHash = await verifier.write.singleTeleport([
  artifacts.proofCalldata,
  artifacts.publicInputs,
  artifacts.treeDepth,
]);
console.log("Teleport tx:", txHash);
```

## Next Steps

- [Receiving](receiving.md) -- scanning for incoming transfers and building proof inputs
- [SDK Quick Start](quickstart.md) -- installation and first private send
- [Wrap and Unwrap](wrap-unwrap.md) -- converting between underlying tokens and zERC20
