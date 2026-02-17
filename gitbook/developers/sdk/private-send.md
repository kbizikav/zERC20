# Private Send

This page walks through each step of a private zERC20 transfer using the SDK.

## How It Works

A private send follows three on/off-chain phases:

1. **Derive a burn address** -- The sender computes a deterministic burn address from the recipient's address and a random secret using Poseidon hashing (with a 16-bit proof-of-work check).
2. **Transfer zERC20 to the burn address** -- A standard ERC-20 `transfer` moves tokens into the burn address. The indexer records the transfer leaf.
3. **Submit an encrypted announcement** -- The sender encrypts transfer metadata (secret, amount, etc.) via the ICP canister so that only the recipient can decrypt and later claim the funds.

> The recipient can then scan the ICP storage canister, decrypt their announcements, and generate a ZKP to mint the equivalent zERC20 via `Verifier.teleport()`.

## Step 1: Derive Seed

Every private send starts with a **seed** -- a wallet-signed message that deterministically derives stealth keys.

```typescript
import { getSeedMessage } from "zerc20-client-sdk";

// getSeedMessage() returns a human-readable string for the wallet to sign
const message = getSeedMessage();
const seedHex = await walletClient.signMessage({ message });
```

`getSeedMessage()` returns a fixed string. The hex-encoded signature (`seedHex`) is used as the entropy source for all subsequent derivations.

## Step 2: Prepare the Private Send

Call `preparePrivateSend()` to derive the burn address, generate the secret, and build the encrypted announcement payload.

```typescript
import { preparePrivateSend } from "zerc20-client-sdk";

const preparation = await preparePrivateSend({
  client,                               // StealthCanisterClient
  recipientAddress: "0xRecipient...",   // EVM address of the recipient
  recipientChainId: 42161n,             // Chain ID where recipient will claim
  seedHex,                              // Hex-encoded wallet signature
});
```

### Parameters

`preparePrivateSend` accepts a single `PreparePrivateSendParams` object:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `client` | `StealthCanisterClient` | Yes | ICP stealth client from `sdk.createStealthClient()` |
| `recipientAddress` | `string` | Yes | Recipient's EVM address |
| `recipientChainId` | `number \| bigint` | Yes | Chain ID the recipient will claim on |
| `seedHex` | `string` | Yes | Hex-encoded wallet signature from Step 1 |
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

Use any EVM library (viem, ethers, etc.) to execute a standard ERC-20 `transfer` to `preparation.burnAddress`.

```typescript
import { encodeFunctionData, erc20Abi } from "viem";

const txHash = await walletClient.sendTransaction({
  to: tokenAddress,   // zERC20 contract address on the sender's chain
  data: encodeFunctionData({
    abi: erc20Abi,
    functionName: "transfer",
    args: [preparation.burnAddress, amount],
  }),
});

// Wait for confirmation
await publicClient.waitForTransactionReceipt({ hash: txHash });
```

> **Important:** The transfer amount is not encoded in the announcement -- the indexer discovers it from the on-chain event. You can transfer any amount in a single transaction.

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
  loadTokens,
  findTokenByChain,
  createProviderForToken,
  getSeedMessage,
  preparePrivateSend,
  submitPrivateSendAnnouncement,
} from "zerc20-client-sdk";
import { createWalletClient, custom, encodeFunctionData, erc20Abi } from "viem";
import { arbitrum } from "viem/chains";

// --- Setup ---
const sdk = createSdk();
const stealthClient = sdk.createStealthClient();

const walletClient = createWalletClient({
  chain: arbitrum,
  transport: custom(window.ethereum!),
});

const { tokens } = await loadTokens();
const entry = findTokenByChain(tokens, 42161n);
const publicClient = createProviderForToken(entry);

// --- Step 1: Derive seed ---
const seedMsg = getSeedMessage();
const [account] = await walletClient.getAddresses();
const seedHex = await walletClient.signMessage({
  account,
  message: seedMsg,
});

// --- Step 2: Prepare ---
const preparation = await preparePrivateSend({
  client: stealthClient,
  recipientAddress: "0xAbC123...def",
  recipientChainId: 42161n,
  seedHex,
});

// --- Step 3: Transfer ---
const txHash = await walletClient.sendTransaction({
  account,
  to: entry.tokenAddress,
  data: encodeFunctionData({
    abi: erc20Abi,
    functionName: "transfer",
    args: [preparation.burnAddress, 100_000_000n], // 100 zUSDC (6 decimals)
  }),
});
await publicClient.waitForTransactionReceipt({ hash: txHash });

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
