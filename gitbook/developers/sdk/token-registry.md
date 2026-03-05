# Token Registry

The SDK includes a token registry module for loading, normalizing, and querying zERC20 token configuration. Token configuration files describe the deployed contracts, chain IDs, and RPC endpoints for each token.

## Loading Tokens

### From compressed data

The production frontend ships tokens as a Base64-encoded gzip string. Use `TokensCacheManager.load()` to decompress and normalize:

```typescript
import { TokensCacheManager } from "zerc20-client-sdk";

const cache = new TokensCacheManager();
const { hub, tokens } = await cache.load(compressedString, {
  cacheKey: "zusdc-main",
});
```

### From raw JSON

If you have a `tokens.json` file (e.g., from your own deployment), use `normalizeTokens`:

```typescript
import { normalizeTokens } from "zerc20-client-sdk";

const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokens(tokensFile);
```

### With RPC URL overrides

Use `normalizeTokensWithOverrides` to replace default RPC URLs at runtime. This is useful for using custom RPC endpoints, private RPCs, or fallback providers:

```typescript
import { normalizeTokensWithOverrides } from "zerc20-client-sdk";

const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokensWithOverrides(tokensFile, {
  tokens: {
    "Arbitrum One": ["https://my-rpc.example.com/arb"],
    "Ethereum": ["https://my-rpc.example.com/eth"],
    "Base": ["https://my-rpc.example.com/base"],
  },
  hub: ["https://my-rpc.example.com/base"],
});
```

The `overrides.tokens` keys are matched against the `label` field in the token configuration. Only matching entries have their RPC URLs replaced.

## Finding Tokens

```typescript
import { findTokenByChain } from "zerc20-client-sdk";

const entry = findTokenByChain(tokens, 42161n); // Arbitrum
// entry.tokenAddress        — zERC20 contract address
// entry.verifierAddress     — Verifier contract address
// entry.liquidityManagerAddress — LiquidityManager address
// entry.rpcUrls             — RPC endpoint URLs
// entry.chainId             — Chain ID (bigint)
```

`findTokenByChain` throws if no token matches the given chain ID.

## Types

### NormalizedTokens

```typescript
interface NormalizedTokens {
  hub?: HubEntry;       // Hub contract metadata (may be undefined for single-chain setups)
  tokens: TokenEntry[]; // Per-chain token entries
  raw: TokensFile;      // Original input, preserved for reference
}
```

### TokenEntry

Each `TokenEntry` represents a token deployment on a specific chain:

| Field | Type | Description |
|-------|------|-------------|
| `tokenAddress` | `string` | zERC20 contract address |
| `verifierAddress` | `string` | Verifier contract address |
| `minterAddress` | `string \| undefined` | Optional minter contract address |
| `liquidityManagerAddress` | `string \| undefined` | Optional LiquidityManager contract address |
| `adaptorAddress` | `string \| undefined` | Optional adaptor contract address (for cross-chain unwrap) |
| `eid` | `number \| undefined` | Optional LayerZero endpoint ID |
| `chainId` | `bigint` | EVM chain ID |
| `deployedBlockNumber` | `bigint` | Deployment block number |
| `label` | `string` | Human-readable chain name (e.g., "Arbitrum One") |
| `rpcUrls` | `string[]` | RPC endpoint URLs for this chain |
| `legacyTx` | `boolean` | Whether to use legacy tx format on this chain |

### HubEntry

| Field | Type | Description |
|-------|------|-------------|
| `hubAddress` | `string` | Hub contract address |
| `chainId` | `bigint` | Chain ID where the Hub is deployed |
| `eid` | `number \| undefined` | Optional LayerZero endpoint ID |
| `rpcUrls` | `string[]` | RPC endpoint URLs |

### RpcOverrides

```typescript
interface RpcOverrides {
  tokens?: Record<string, string[]>;
  hub?: string[];
}
```

## Function Signatures

```typescript
function normalizeTokens(file: TokensFile): NormalizedTokens;

function normalizeTokensWithOverrides(
  file: TokensFile,
  overrides?: RpcOverrides,
): NormalizedTokens;

function findTokenByChain(
  tokens: readonly TokenEntry[],
  chainId: bigint,
): TokenEntry;
```

## See Also

- [SDK Quick Start](quickstart.md) -- installation and first token load
- [Contract Addresses](../../reference/addresses.md) -- deployed addresses per chain
