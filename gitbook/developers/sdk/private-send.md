# Private Send

This page walks through each step of a private zERC20 transfer using the SDK.

## How It Works

A private send follows three on/off-chain phases:

1. **Derive a burn address** -- The sender computes a deterministic burn address from the recipient's address and a random secret using Poseidon hashing (with a 16-bit proof-of-work check).
2. **Transfer zERC20 to the burn address** -- A standard ERC-20 `transfer` moves tokens into the burn address. The indexer records the transfer leaf.
3. **Submit an encrypted announcement** -- The sender encrypts transfer metadata (secret, amount, etc.) via the ICP canister so that only the recipient can decrypt and later claim the funds.

> The recipient can then scan the ICP storage canister, decrypt their announcements, and generate a ZKP to mint the equivalent zERC20 via `Verifier.teleport()`.

## Optional: Pre-flight Blocklist Check

zERC20 tokens enforce the on-chain [Blocklist](../specs/contract-spec.md#blocklist) at `transfer` time. Any transfer to a blocked recipient will revert with `AddressIsBlocked`, and because the private-send path sends to a burn address derived from the recipient, the funds would be unrecoverable if the recipient later turned out to be sanctioned.

Call `isBlockedAddress()` before preparing a send to fail fast with a user-friendly error instead of a reverted transaction:

```typescript
import { isBlockedAddress } from "zerc20-client-sdk";

const blocked = await isBlockedAddress(readProvider, blocklistAddress, recipientAddress);
if (blocked) {
  throw new Error("Recipient is on the OFAC sanctions blocklist");
}
```

The blocklist contract address is shared across all zERC20 tokens on the same chain; see [Addresses](../../reference/addresses.md) for deployed addresses.

## Step 1: Derive Seed

Every private send starts with a **seed** -- a wallet-signed message that deterministically derives stealth keys.

```typescript
import { getSeedMessage } from "zerc20-client-sdk";
import { keccak256, toBytes } from "viem";

// getSeedMessage() is async and returns a human-readable string for the wallet to sign
const message = await getSeedMessage();
const signature = await walletClient.signMessage({ message });

// Hash the 65-byte signature down to 32 bytes -- the SDK requires a 32-byte hex seed
const seedHex = keccak256(toBytes(signature));
```

`getSeedMessage()` returns an async `Promise<string>`. The wallet signature (65 bytes) must be hashed with `keccak256` to produce a 32-byte hex string for `seedHex`. The SDK validates that `seedHex` is exactly 32 bytes and will throw if it is not.

## Step 2: Prepare the Private Send

Call `preparePrivateSend()` to derive the burn address, generate the secret, and build the encrypted announcement payload.

```typescript
import { preparePrivateSend } from "zerc20-client-sdk";

const preparation = await preparePrivateSend({
  client,                               // StealthCanisterClient
  recipientAddress: "0xRecipient...",   // EVM address of the recipient
  recipientChainId: 42161n,             // Chain ID where recipient will claim
  seedHex,                              // 32-byte hex seed (keccak256 of signature)
});
```

### Parameters

`preparePrivateSend` accepts a single `PreparePrivateSendParams` object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `client` | `StealthCanisterClient` | Yes | ICP stealth client from `sdk.createStealthClient()` |
| `recipientAddress` | `string` | Yes | Recipient's EVM address |
| `recipientChainId` | `number \| bigint` | Yes | Chain ID the recipient will claim on |
| `seedHex` | `string` | Yes | 32-byte hex string (`keccak256` of the wallet signature from Step 1) |
| `paymentAdviceIdHex` | `string` | No | Optional payment-advice identifier |
| `vetkdKeyIdName` | `string` | No | Override VetKD key ID name |

### Return Value

`preparePrivateSend` returns a `PreparedPrivateSend` object:

| Field | Type | Description |
|-------|------|-------------|
| `burnAddress` | `string` | Deterministic address to send zERC20 to |
| `burnPayload` | `Uint8Array` | Serialized burn data |
| `secret` | `bigint` | Random secret bound to the burn address |
| `tweak` | `bigint` | Poseidon-derived tweak value |
| `generalRecipient` | `string` | Generalized recipient identifier |
| `announcement` | `object` | Encrypted announcement ready for submission |
| `sessionKey` | `Uint8Array` | Ephemeral session key |
| `paymentAdviceId` | `string` | Resolved payment-advice identifier |
| `paymentAdviceIdBytes` | `Uint8Array` | Payment-advice identifier as bytes |

### Signature

```typescript
function preparePrivateSend(
  params: PreparePrivateSendParams,
): Promise<PreparedPrivateSend>;
```

## Step 3: Transfer zERC20 to the Burn Address

Use the SDK's `submitPrivateSendTransfer()` to execute the ERC-20 transfer to the burn address. This helper encapsulates the `transfer` call using the library-agnostic `EvmWriteProvider` interface.

```typescript
import { submitPrivateSendTransfer } from "zerc20-client-sdk";

const { transactionHash } = await submitPrivateSendTransfer({
  writeProvider,                           // EvmWriteProvider (e.g., viem WalletClient)
  tokenAddress: entry.tokenAddress,        // zERC20 contract address
  burnAddress: preparation.burnAddress,    // from Step 2
  amount: 100_000_000n,                    // 100 zUSDC (6 decimals)
  readProvider,                            // optional: EvmReadProvider for receipt polling
});
```

### Parameters

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `writeProvider` | `EvmWriteProvider` | Yes | Wallet provider to sign and send the transaction |
| `tokenAddress` | `string` | Yes | zERC20 contract address on the sender's chain |
| `burnAddress` | `string` | Yes | Burn address from `preparePrivateSend()` |
| `amount` | `bigint` | Yes | Amount to transfer in the token's smallest unit |
| `feeOverrides` | `FeeOverrides` | No | Optional gas-price overrides from `buildFeeOverrides` |
| `readProvider` | `EvmReadProvider` | No | Provider for receipt polling; falls back to `writeProvider` |

### Return Value

| Field | Type | Description |
|-------|------|-------------|
| `transactionHash` | `Hex` | Confirmed transaction hash |

> **Note:** You can also execute the transfer manually using any EVM library -- `submitPrivateSendTransfer` is a convenience wrapper. The important thing is that a standard ERC-20 `transfer(burnAddress, amount)` call reaches the zERC20 contract.

### Signature

```typescript
function submitPrivateSendTransfer(
  params: SubmitPrivateSendTransferParams,
): Promise<{ transactionHash: Hex }>;
```

## Step 4: Submit the Announcement

After the on-chain transfer is confirmed, submit the encrypted announcement to the ICP storage canister so the recipient can discover the transfer.

```typescript
import { submitPrivateSendAnnouncement } from "zerc20-client-sdk";

const result = await submitPrivateSendAnnouncement({
  client,       // StealthCanisterClient
  preparation,  // PreparedPrivateSend from Step 2
  tag: "myApp", // Optional application-level tag
});
```

### Parameters

`submitPrivateSendAnnouncement` accepts a `SubmitPrivateSendParams` object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `client` | `StealthCanisterClient` | Yes | ICP stealth client |
| `preparation` | `PreparedPrivateSend` | Yes | The object returned by `preparePrivateSend()` |
| `tag` | `string` | No | Optional tag for filtering announcements |

### Return Value

Returns a `PrivateSendResult` confirming that the announcement was persisted.

### Signature

```typescript
function submitPrivateSendAnnouncement(
  params: SubmitPrivateSendParams,
): Promise<PrivateSendResult>;
```

## Complete Example

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
import { createWalletClient, createPublicClient, custom, http, keccak256, toBytes } from "viem";
import { arbitrum } from "viem/chains";
import { HttpAgent } from "@dfinity/agent";

// --- Setup ---
const sdk = createSdk();
const agent = await HttpAgent.create({ host: "https://icp-api.io" });
const stealthClient = sdk.createStealthClient({
  agent,
  storageCanisterId: "your-storage-canister-id",
  keyManagerCanisterId: "your-key-manager-canister-id",
});

const writeProvider = createWalletClient({
  chain: arbitrum,
  transport: custom(window.ethereum!),
});

// Load your tokens (from your own config or built-in compressed data)
const tokensFile = await import("./tokens.json");
const { tokens } = normalizeTokens(tokensFile);
const entry = findTokenByChain(tokens, 42161n);

const readProvider = createPublicClient({
  chain: arbitrum,
  transport: http(entry.rpcUrls[0]),
});

// --- Step 1: Derive seed ---
const seedMsg = await getSeedMessage();
const [account] = await writeProvider.getAddresses();
const signature = await writeProvider.signMessage({
  account,
  message: seedMsg,
});
const seedHex = keccak256(toBytes(signature));

// --- Step 2: Prepare ---
const preparation = await preparePrivateSend({
  client: stealthClient,
  recipientAddress: "0xAbC123...def",
  recipientChainId: 42161n,
  seedHex,
});

// --- Step 3: Transfer ---
const { transactionHash } = await submitPrivateSendTransfer({
  writeProvider,
  readProvider,
  tokenAddress: entry.tokenAddress,
  burnAddress: preparation.burnAddress,
  amount: 100_000_000n, // 100 zUSDC (6 decimals)
});

// --- Step 4: Submit announcement ---
const result = await submitPrivateSendAnnouncement({
  client: stealthClient,
  preparation,
});

console.log("Private send complete:", result);
```

## Error Handling

| Error | Cause | Resolution |
|-------|-------|------------|
| `SeedSignatureRejected` | User rejected the wallet signature prompt | Prompt the user to sign again; the seed message is deterministic and safe to sign |
| `BurnAddressPoWFailed` | Proof-of-work check on the derived burn address did not pass | Retry `preparePrivateSend()` -- a new secret will be sampled |
| `StealthClientNotConnected` | `createStealthClient()` was not called or the ICP agent is unreachable | Verify the ICP agent host and canister IDs |
| `AnnouncementSubmissionFailed` | The ICP storage canister rejected the announcement | Check that the canister is available and the announcement payload is well-formed |
| `InsufficientBalance` | The sender does not hold enough zERC20 on the source chain | Wrap more underlying tokens via `LiquidityManager.wrap()` or bridge from another chain |
| `TransactionReverted` | The ERC-20 `transfer` call reverted on-chain | Verify token approval, balance, and that the burn address is valid |

## See Also

- [SDK Quick Start](quickstart.md) -- installation and first steps
- [Receiving](receiving.md) -- scanning announcements and claiming funds
- [Architecture Overview](../architecture.md) -- system-level design
- [ZKP Spec](../specs/zkp-spec.md) -- details on Nova and Groth16 proofs
