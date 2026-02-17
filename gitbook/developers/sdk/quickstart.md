# SDK Quick Start

Get started with `zerc20-client-sdk` to send private zERC20 transfers from your TypeScript application.

## Prerequisites

| Requirement | Details |
|-------------|---------|
| **Node.js** | >= 18 |
| **EVM Wallet** | Any wallet that can sign messages (e.g., MetaMask, Rabby) |
| **Supported Chain** | Ethereum, Arbitrum, or Base |

## Installation

```bash
npm install zerc20-client-sdk
```

## Initialize the SDK

Call `createSdk()` to obtain a `Zerc20Sdk` instance. All options are optional and fall back to sensible defaults.

```typescript
import { createSdk } from "zerc20-client-sdk";

const sdk = createSdk();
```

`Zerc20SdkOptions` accepts the following optional fields:

| Field | Type | Description |
|-------|------|-------------|
| `wasm` | `WasmRuntime` | Custom WASM runtime (auto-detected if omitted) |
| `teleportProofs` | `TeleportProofClient` | Custom teleport-proof client |
| `decider` | `HttpDeciderClient` | Decider prover endpoint |
| `stealth` | `StealthClientFactory` | Custom stealth client factory |

The returned `Zerc20Sdk` exposes the same fields for direct access:

```typescript
interface Zerc20Sdk {
  wasm: WasmRuntime;
  teleportProofs: TeleportProofClient;
  decider?: HttpDeciderClient;
  stealth: StealthClientFactory;
}
```

## Connect to ICP

Create a `StealthCanisterClient` to interact with the ICP storage and key-manager canisters:

```typescript
import { HttpAgent } from "@dfinity/agent";

const agent = await HttpAgent.create({ host: "https://icp-api.io" });

const stealthClient = sdk.createStealthClient({
  agent,
  storageCanisterId: "your-storage-canister-id",
  keyManagerCanisterId: "your-key-manager-canister-id",
});
```

The `agent`, `storageCanisterId`, and `keyManagerCanisterId` fields are **required**. If any are missing (and no factory defaults have been set), `createStealthClient()` will throw an error.

```typescript
sdk.createStealthClient(config?: Partial<StealthClientConfig>): StealthCanisterClient
```

## Load Tokens

The SDK provides two ways to load token configuration:

### Option A: From compressed data with `loadTokens`

`loadTokens(compressed)` accepts a **Base64-encoded gzip** string containing token configuration JSON. This is the format used by the production frontend.

```typescript
import {
  loadTokens,
  findTokenByChain,
  createProviderForToken,
} from "zerc20-client-sdk";

// `compressed` is a Base64-encoded gzip string of your tokens.json
const { hub, tokens } = await loadTokens(compressedTokensString);
```

### Option B: From raw JSON with `normalizeTokens`

If you have a `tokens.json` file (e.g., from your own deployment), use `normalizeTokens()` instead:

```typescript
import {
  normalizeTokens,
  findTokenByChain,
  createProviderForToken,
} from "zerc20-client-sdk";

const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokens(tokensFile);
// hub?: HubEntry           -- Hub contract metadata (Base chain)
// tokens: TokenEntry[]     -- per-chain token entries
```

### Finding tokens and creating providers

```typescript
// Pick the token entry for Arbitrum (chain ID 42161)
const entry = findTokenByChain(tokens, 42161n);

// Create a viem PublicClient pre-configured for that chain
const publicClient = createProviderForToken(entry);
```

### Type signatures

```typescript
function loadTokens(compressed: string, options?: LoadTokensOptions): Promise<NormalizedTokens>;
function normalizeTokens(file: TokensFile): NormalizedTokens;

interface NormalizedTokens {
  hub?: HubEntry;
  tokens: TokenEntry[];
  raw: TokensFile;
}

function findTokenByChain(
  tokens: readonly TokenEntry[],
  chainId: bigint,
): TokenEntry;

function createProviderForToken(entry: TokenEntry): PublicClient;
```

## First Private Send

Below is a compact end-to-end example. Each helper is covered in detail on the [Private Send](private-send.md) page.

```typescript
import {
  createSdk,
  normalizeTokens,
  findTokenByChain,
  createProviderForToken,
  getSeedMessage,
  preparePrivateSend,
  submitPrivateSendAnnouncement,
} from "zerc20-client-sdk";
import { encodeFunctionData, erc20Abi, keccak256, toBytes } from "viem";
import { HttpAgent } from "@dfinity/agent";

// 1. Initialize
const sdk = createSdk();
const agent = await HttpAgent.create({ host: "https://icp-api.io" });
const client = sdk.createStealthClient({
  agent,
  storageCanisterId: "your-storage-canister-id",
  keyManagerCanisterId: "your-key-manager-canister-id",
});
const tokensFile = await import("./tokens.json");
const { tokens } = normalizeTokens(tokensFile);
const entry = findTokenByChain(tokens, 42161n);   // Arbitrum

// 2. Derive seed from wallet signature (hash to 32 bytes)
const seedMsg = await getSeedMessage();
const signature = await walletClient.signMessage({ message: seedMsg });
const seedHex = keccak256(toBytes(signature));

// 3. Prepare the private send
const preparation = await preparePrivateSend({
  client,
  recipientAddress: "0xRecipient...",
  recipientChainId: 42161n,
  seedHex,
});

// 4. Transfer zERC20 to the burn address
const txHash = await walletClient.sendTransaction({
  to: entry.tokenAddress,
  data: encodeFunctionData({
    abi: erc20Abi,
    functionName: "transfer",
    args: [preparation.burnAddress, 100_000_000n], // 100 zUSDC
  }),
});

// 5. Submit the encrypted announcement
const result = await submitPrivateSendAnnouncement({
  client,
  preparation,
});
console.log("Announcement submitted:", result);
```

## Next Steps

- [Private Send](private-send.md) -- detailed walkthrough of each step
- [Receiving](receiving.md) -- scanning and claiming incoming transfers
- [Integration Guide](../integration.md) -- on-chain oracle and self-hosted indexer
- [Contract Addresses](../../reference/addresses.md) -- deployed addresses per chain
