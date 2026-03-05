# LayerZero Scan

The SDK includes a LayerZero Scan module for tracking cross-chain messages. This is useful for building transaction history UIs, monitoring bridge progress, and decoding OFT (Omnichain Fungible Token) payloads.

## Overview

When zERC20 tokens are transferred cross-chain (e.g., cross-chain unwrap via Stargate), the transaction goes through LayerZero's messaging protocol. The LayerZero Scan module provides:

- **API client** -- Query the LayerZero Scan API for wallet messages and transaction messages
- **Payload decoding** -- Decode OFT `send()` calldata, compose messages, and bridge requests
- **Status orchestration** -- High-level `fetchWalletStatus()` that fetches, filters, and decodes all messages for a wallet

## Configuration

The module requires a `LayerZeroScanConfig` object instead of reading from environment variables. This allows framework-agnostic usage:

```typescript
import type { LayerZeroScanConfig } from "zerc20-client-sdk";

const scanConfig: LayerZeroScanConfig = {
  baseUrl: "https://scan.layerzero-api.com",
  apiKey: "your-api-key",  // optional
};
```

## Fetch Wallet Status

The primary entry point. Fetches all LayerZero messages for a wallet, filters by token, and decodes payloads:

```typescript
import { fetchWalletStatus } from "zerc20-client-sdk";
import type { EvmReadProvider } from "zerc20-client-sdk";

// createReadProvider is a factory that returns an EvmReadProvider for a given token
function createReadProvider(token: TokenEntry): EvmReadProvider {
  return createPublicClient({ chain: ..., transport: http(token.rpcUrls[0]) });
}

const result = await fetchWalletStatus({
  address: "0xYourWallet...",
  tokens,                           // TokenEntry[] from normalizeTokens
  scanConfig: {
    baseUrl: "https://scan.layerzero-api.com",
    apiKey: "your-api-key",
  },
  createReadProvider,               // (token: TokenEntry) => EvmReadProvider
  limit: 20,                        // optional, messages per page
  nextToken: undefined,             // optional, pagination cursor
  filterByToken: true,              // optional, filter messages by token addresses
});
```

### FetchWalletStatusParams

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `address` | `string` | Yes | Wallet address to query |
| `tokens` | `TokenEntry[]` | Yes | Token configuration for filtering and decoding |
| `scanConfig` | `LayerZeroScanConfig` | Yes | API base URL and optional API key |
| `createReadProvider` | `(token: TokenEntry) => EvmReadProvider` | Yes | Factory to create read providers per token |
| `limit` | `number` | No | Messages per page (default: 25) |
| `nextToken` | `string` | No | Pagination cursor from a previous response |
| `filterByToken` | `boolean` | No | Filter messages to only those involving the configured tokens |

### WalletStatusResult

| Field | Type | Description |
|-------|------|-------------|
| `messages` | `LayerZeroMessageSummary[]` | Decoded message summaries |
| `nextToken` | `string \| undefined` | Pagination cursor for the next page |

### LayerZeroMessageSummary

Each decoded message contains:

| Field | Type | Description |
|-------|------|-------------|
| `guid` | `string` | Unique message identifier |
| `pathway` | `object` | Source and destination chain info (`src`, `dst`, `nonce`, `srcEid`, `dstEid`) |
| `sourceTx` | `string` | Source transaction hash |
| `sourceBlock` | `string` | Source block info |
| `destinationTx` | `string \| undefined` | Destination transaction hash (once delivered) |
| `send` | `SendPayloadSummary \| null` | Decoded send payload (amounts, addresses) |
| `composeFollowups` | `ComposeFollowupSummary[]` | Compose message follow-ups (e.g., bridge requests) |
| `status` | `string` | Message status (e.g., "DELIVERED", "INFLIGHT") |
| `raw` | `LayerZeroScanMessage` | Original API response |

## Low-Level API

### Scan API Client

Query the LayerZero Scan API directly:

```typescript
import { fetchWalletMessages, fetchTxMessages, getWalletMessagesUrl } from "zerc20-client-sdk";

// Fetch messages for a wallet
const response = await fetchWalletMessages(scanConfig, "0xWallet...", {
  limit: 10,
  nextToken: undefined,
});

// Fetch messages for a specific transaction
const txMessages = await fetchTxMessages(scanConfig, "0xTxHash...");

// Build LZ Scan explorer URL
const url = getWalletMessagesUrl(scanConfig, "0xWallet...");
```

### Payload Decoding

Decode OFT send payloads and compose messages:

```typescript
import { decodeSendSummary, tryDecodeBridgeRequest, fetchOftSentAmount } from "zerc20-client-sdk";

// Decode an OFT send() calldata
const summary = decodeSendSummary(txInputData);
// → { dstEid, to, amount, minAmount, compose, source: "tx" | "payload" }

// Decode a compose message as a bridge request
const bridge = tryDecodeBridgeRequest(composeMsg);
// → { dstEid, to, refundAddress, minAmountOut } | null

// Read OFTSent event amount from transaction logs
const amount = await fetchOftSentAmount(readProvider, txHash);
```

### Formatting Utilities

```typescript
import { endpointChain, destinationTx, formatPathway, isMessageForTokens } from "zerc20-client-sdk";

// Get chain name from LZ endpoint
const chain = endpointChain(message.pathway?.sender, "src");

// Extract destination transaction hash
const dstTx = destinationTx(message);

// Format pathway string (e.g., "Arbitrum → Ethereum")
const path = formatPathway(message);

// Check if message involves configured tokens
const relevant = isMessageForTokens(message, tokens);
```

## See Also

- [SDK Quick Start](quickstart.md) -- installation and setup
- [Wrap and Unwrap](wrap-unwrap.md) -- cross-chain unwrap triggers LayerZero messages
- [API Reference](api-reference.md) -- complete function listing
