# Token Registry

The SDK includes a token registry module for loading, normalizing, and querying zERC20 token configuration. Token configuration files describe the deployed contracts, chain IDs, and RPC endpoints for each token.

## Loading Tokens

### From compressed data

The production frontend ships tokens as a Base64-encoded gzip string. Use `TokensCacheManager` (or the underlying `loadTokens` helper) to decompress and normalize:

```typescript
import { TokensCacheManager } from "zerc20-client-sdk";

const cache = new TokensCacheManager();
const { hub, tokens } = await cache.loadTokensForSymbol("zusdc", compressedString);
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
  "Arbitrum One": "https://my-rpc.example.com/arb",
  "Ethereum":    "https://my-rpc.example.com/eth",
  "Base":        "https://my-rpc.example.com/base",
});
```

The override keys are matched against the `label` field in the token configuration. Only matching entries have their RPC URLs replaced.

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
| `liquidityManagerAddress` | `string` | LiquidityManager contract address |
| `adaptorAddress` | `string` | Adaptor contract address (for cross-chain unwrap) |
| `chainId` | `bigint` | EVM chain ID |
| `label` | `string` | Human-readable chain name (e.g., "Arbitrum One") |
| `rpcUrls` | `string[]` | RPC endpoint URLs for this chain |
| `indexerUrl` | `string` | Indexer HTTP endpoint |
| `deciderUrl` | `string` | Decider service endpoint |
| `decimals` | `number` | Token decimals |

### HubEntry

| Field | Type | Description |
|-------|------|-------------|
| `hubAddress` | `string` | Hub contract address |
| `chainId` | `bigint` | Chain ID where the Hub is deployed |
| `rpcUrls` | `string[]` | RPC endpoint URLs |

### RpcOverrides

```typescript
type RpcOverrides = Record<string, string>;
// Key: chain label (e.g., "Arbitrum One")
// Value: replacement RPC URL
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
