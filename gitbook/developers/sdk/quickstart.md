# SDK Quick Start

Get started with `zerc20-client-sdk` to send private zERC20 transfers from your TypeScript application.

## Prerequisites

| Requirement | Details |
|-------------|---------|
| **Node.js** | >= 18 |
| **EVM Wallet** | Any wallet that can sign messages (e.g., MetaMask, Rabby) |
| **Supported Chain** | Ethereum, Arbitrum, Base, BNB Chain |

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
| `proofs` | `ProofService` | Custom proof service |
| `decider` | `HttpDeciderClient` | Decider prover endpoint |
| `stealth` | `StealthClientFactory` | Custom stealth client factory |

The returned `Zerc20Sdk` exposes the same fields for direct access:

```typescript
interface Zerc20Sdk {
  wasm: WasmRuntime;
  proofs: ProofService;
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

## EVM Providers

The SDK uses library-agnostic provider interfaces instead of depending directly on viem. This means you can use viem, ethers.js, or any other EVM library:

| Interface | Purpose | viem Equivalent |
|-----------|---------|-----------------|
| `EvmReadProvider` | Contract reads, balance queries, fee estimation | `PublicClient` |
| `EvmWriteProvider` | Signing and submitting transactions | `WalletClient` |

viem's `PublicClient` and `WalletClient` satisfy these interfaces structurally -- no adapter is needed. For other libraries, provide a thin adapter that implements the required methods.

```typescript
import type { EvmReadProvider, EvmWriteProvider } from "zerc20-client-sdk";
import { createPublicClient, createWalletClient, custom, http } from "viem";
import { arbitrum } from "viem/chains";

// These work as EvmReadProvider / EvmWriteProvider directly
const readProvider = createPublicClient({ chain: arbitrum, transport: http() });
const writeProvider = createWalletClient({ chain: arbitrum, transport: custom(window.ethereum!) });
```

## Load Tokens

The SDK provides several ways to load token configuration:

### Option A: From compressed data with `TokensCacheManager`

`TokensCacheManager.load(compressed)` accepts a **Base64-encoded gzip** string containing token configuration JSON. This is the format used by the production frontend.

```typescript
import { TokensCacheManager, findTokenByChain } from "zerc20-client-sdk";

// `compressed` is a Base64-encoded gzip string of your tokens.json
const cache = new TokensCacheManager();
const { hub, tokens } = await cache.load(compressedTokensString, {
  cacheKey: "zusdc-main",
});
```

### Option B: From raw JSON with `normalizeTokens`

If you have a `tokens.json` file (e.g., from your own deployment), use `normalizeTokens()`:

```typescript
import { normalizeTokens, findTokenByChain } from "zerc20-client-sdk";

const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokens(tokensFile);
// hub?: HubEntry           -- Hub contract metadata (Base chain)
// tokens: TokenEntry[]     -- per-chain token entries
```

### Option C: With RPC URL overrides

Use `normalizeTokensWithOverrides()` to replace default RPC URLs at runtime:

```typescript
import { normalizeTokensWithOverrides, findTokenByChain } from "zerc20-client-sdk";

const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokensWithOverrides(tokensFile, {
  tokens: {
    "Arbitrum One": ["https://my-rpc.example.com/arb"],
    "Ethereum": ["https://my-rpc.example.com/eth"],
  },
  hub: ["https://my-rpc.example.com/base"],
});
```

### Finding tokens

```typescript
// Pick the token entry for Arbitrum (chain ID 42161)
const entry = findTokenByChain(tokens, 42161n);
// entry.tokenAddress, entry.liquidityManagerAddress, etc.
```

See [Token Registry](token-registry.md) for the full token API.

### Type signatures

```typescript
function normalizeTokens(file: TokensFile): NormalizedTokens;
function normalizeTokensWithOverrides(file: TokensFile, overrides?: RpcOverrides): NormalizedTokens;

interface NormalizedTokens {
  hub?: HubEntry;
  tokens: TokenEntry[];
  raw: TokensFile;
}

function findTokenByChain(
  tokens: readonly TokenEntry[],
  chainId: bigint,
): TokenEntry;
```

## First Private Send

Below is a compact end-to-end example. Each helper is covered in detail on the [Private Send](private-send.md) page.

```typescript
import {
  createSdk,
  normalizeTokens,
  findTokenByChain,
  getSeedMessage,
  preparePrivateSend,
  submitPrivateSendAnnouncement,
  submitPrivateSendTransfer,
} from "zerc20-client-sdk";
import { createPublicClient, createWalletClient, custom, http, keccak256, toBytes } from "viem";
import { arbitrum } from "viem/chains";
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

// 2. Create providers
const readProvider = createPublicClient({ chain: arbitrum, transport: http(entry.rpcUrls[0]) });
const writeProvider = createWalletClient({ chain: arbitrum, transport: custom(window.ethereum!) });

// 3. Derive seed from wallet signature (hash to 32 bytes)
const seedMsg = await getSeedMessage();
const signature = await writeProvider.signMessage({ message: seedMsg });
const seedHex = keccak256(toBytes(signature));

// 4. Prepare the private send
const preparation = await preparePrivateSend({
  client,
  recipientAddress: "0xRecipient...",
  recipientChainId: 42161n,
  seedHex,
});

// 5. Transfer zERC20 to the burn address
const { transactionHash } = await submitPrivateSendTransfer({
  writeProvider,
  readProvider,
  tokenAddress: entry.tokenAddress,
  burnAddress: preparation.burnAddress,
  amount: 100_000_000n,  // 100 zUSDC (6 decimals)
});

// 6. Submit the encrypted announcement
const result = await submitPrivateSendAnnouncement({
  client,
  preparation,
});
console.log("Announcement submitted:", result);
```

## Next Steps

- [Private Send](private-send.md) -- detailed walkthrough of each step
- [Receiving](receiving.md) -- scanning and claiming incoming transfers
- [API Reference](api-reference.md#relay) -- relay-node helpers for gasless redeem and native-gas swaps
- [Token Registry](token-registry.md) -- loading and querying token configuration
- [Integration Guide](../integration.md) -- on-chain oracle and self-hosted indexer
- [Contract Addresses](../../reference/addresses.md) -- deployed addresses per chain
