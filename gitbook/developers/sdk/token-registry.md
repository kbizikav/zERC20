# Token Registry

The SDK ships with compressed token configuration for mainnet and testnet deployments. The registry module loads, normalizes, caches, and provides helpers for finding tokens and creating RPC providers.

## Config Format

### `TokenEntry`

| Field | Type | Description |
|-------|------|-------------|
| `label` | `string` | Human-readable label (e.g., "eth-mainnet") |
| `tokenAddress` | `string` | zERC20 token contract address |
| `verifierAddress` | `string` | Verifier contract address |
| `minterAddress` | `string?` | Optional minter contract address |
| `liquidityManagerAddress` | `string?` | Optional LiquidityManager address |
| `adaptorAddress` | `string?` | Optional LayerZero Adaptor address |
| `eid` | `number?` | Optional LayerZero endpoint ID |
| `chainId` | `bigint` | EVM chain ID |
| `deployedBlockNumber` | `bigint` | Block number of token deployment |
| `rpcUrls` | `string[]` | RPC URLs for this chain |
| `legacyTx` | `boolean` | Use legacy (type-0) transactions |

### `HubEntry`

| Field | Type | Description |
|-------|------|-------------|
| `hubAddress` | `string` | Hub contract address |
| `chainId` | `bigint` | Hub chain ID |
| `eid` | `number?` | Optional LayerZero endpoint ID |
| `rpcUrls` | `string[]` | RPC URLs for hub chain |

## Loading Tokens

### `loadTokens`

```typescript
loadTokens(compressed: string, options?: LoadTokensOptions): Promise<NormalizedTokens>
```

Loads token configuration from a compressed string. The `compressed` argument must be a **Base64-encoded gzip** payload whose decompressed content is UTF-8 JSON conforming to the `TokensFile` schema. Results are cached by default so repeated calls with the same payload are cheap.

**Returns** `{ hub?: HubEntry, tokens: TokenEntry[], raw: TokensFile }`

- `hub` -- The hub entry, if one is defined in the configuration.
- `tokens` -- An array of all token entries with fields normalized to their expected types (`bigint` chain IDs, etc.).
- `raw` -- The original deserialized `TokensFile` before normalization.

### `normalizeTokens`

```typescript
normalizeTokens(file: TokensFile): NormalizedTokens
```

Normalizes a raw JSON tokens file. Use this when you have a custom deployment and want to supply your own `TokensFile` object rather than using the built-in data.

**Returns** `NormalizedTokens` -- the same shape as the `loadTokens` return value.

## Finding Tokens

### `findTokenByChain`

```typescript
findTokenByChain(tokens: readonly TokenEntry[], chainId: bigint): TokenEntry
```

Finds a single `TokenEntry` whose `chainId` matches the given value. Throws if no entry is found or if multiple entries match (which would indicate a misconfigured token file).

## RPC Providers

### `createProviderForToken`

```typescript
createProviderForToken(entry: TokenEntry): PublicClient
```

Creates a viem `PublicClient` using the first RPC URL in the token entry's `rpcUrls` array. The returned client is configured with the correct chain ID and is ready for read operations against that chain.

## Caching

The registry module caches the result of `loadTokens` so that repeated calls do not decompress and parse the built-in data more than once.

### `TokensCacheManager`

A class that manages cached token loads. You rarely need to interact with it directly; the module maintains a default singleton instance.

### `clearTokensCache`

```typescript
clearTokensCache(): void
```

Clears the cached entries in the default cache manager. The next call to `loadTokens` will decompress and parse the data again.

### `resetTokensCache`

```typescript
resetTokensCache(): void
```

Resets the cache manager entirely, replacing the default singleton with a fresh instance.

### `getDefaultTokensCacheManager`

```typescript
getDefaultTokensCacheManager(): TokensCacheManager
```

Returns the singleton `TokensCacheManager` used by `loadTokens` when no custom cache manager is provided.

## Example

```typescript
import {
  normalizeTokens,
  findTokenByChain,
  createProviderForToken,
} from "zerc20-client-sdk";

// 1. Load token configuration from your tokens.json
const tokensFile = await import("./tokens.json");
const { hub, tokens } = normalizeTokens(tokensFile);

console.log(`Loaded ${tokens.length} token(s)`);
if (hub) {
  console.log(`Hub on chain ${hub.chainId}: ${hub.hubAddress}`);
}

// 2. Find the token entry for Ethereum mainnet (chain ID 1)
const ethToken = findTokenByChain(tokens, 1n);
console.log(`zERC20 on Ethereum: ${ethToken.tokenAddress}`);
console.log(`Verifier: ${ethToken.verifierAddress}`);

// 3. Create a viem PublicClient for that chain
const provider = createProviderForToken(ethToken);
const blockNumber = await provider.getBlockNumber();
console.log(`Current block: ${blockNumber}`);
```
