# Invoices

Invoices allow a recipient to pre-generate burn addresses and share them with a sender, so the sender knows exactly where to transfer tokens. This is the recommended way to coordinate private transfers between parties.

## What Are Invoices

An invoice encapsulates one or more burn addresses that a recipient generates ahead of time. When a sender receives an invoice, they transfer zERC20 tokens to the burn address(es) listed in it.

There are two invoice types:

| Type       | Burn Addresses | Use Case                                      |
| ---------- | -------------- | --------------------------------------------- |
| **Single** | 1              | One-time payment to a single burn address     |
| **Batch**  | 10             | Multiple payments or splitting across amounts |

Batch invoices always contain exactly `NUM_BATCH_INVOICES = 10` burn addresses.

Each burn address in an invoice is derived from the recipient's address, a secret, and a tweak, ensuring that only the recipient can later redeem the tokens sent to it.

## Prepare Invoice

Generate the invoice artifacts locally before submitting to the ICP storage canister. This step derives the burn addresses and prepares a signature message for wallet signing.

```typescript
import { prepareInvoiceIssue } from "zerc20-client-sdk";

const artifacts = await prepareInvoiceIssue({
  client,            // StealthCanisterClient
  seedHex,           // 32-byte hex seed (keccak256 of wallet signature)
  recipientAddress,  // recipient EVM address
  recipientChainId,  // chain ID where the recipient will redeem
  isBatch: false,    // true for batch (10 addresses), false for single
  tag: undefined,    // optional, tag for filtering
  randomBytes: undefined, // optional, custom random bytes
  maxRetries: undefined,  // optional, max PoW retries
});
```

**Signature:**

```typescript
function prepareInvoiceIssue(
  params: InvoiceIssueParams,
): Promise<InvoiceIssueArtifacts>;
```

**InvoiceIssueParams:**

| Field              | Type                    | Required | Description                                    |
| ------------------ | ----------------------- | -------- | ---------------------------------------------- |
| `client`           | `StealthCanisterClient` | Yes      | ICP canister client                            |
| `seedHex`          | `string`                | Yes      | 32-byte hex seed (see [Private Send](private-send.md#step-1-derive-seed) for derivation)  |
| `recipientAddress` | `string`                | Yes      | Recipient EVM address                          |
| `recipientChainId` | `number \| bigint`      | Yes      | Chain ID where the recipient will redeem       |
| `isBatch`          | `boolean`               | Yes      | `true` for batch (10 addresses), `false` for single |
| `tag`              | `string \| undefined`   | No       | Tag for categorizing or filtering invoices     |
| `randomBytes`      | `(length: number) => Uint8Array` | No | Custom random bytes generator for burn address derivation |
| `maxRetries`       | `number \| undefined`   | No       | Max PoW retries for burn address generation    |

**InvoiceIssueArtifacts:**

| Field              | Type                         | Description                                     |
| ------------------ | ---------------------------- | ----------------------------------------------- |
| `invoiceId`        | `string`                     | Unique invoice identifier                       |
| `recipientAddress` | `string`                     | Recipient EVM address                           |
| `recipientChainId` | `bigint`                     | Chain ID for redemption                         |
| `burnAddresses`    | `InvoiceBatchBurnAddress[]`  | Array of burn address entries                   |
| `signatureMessage` | `string`                     | Message to sign with the recipient's wallet     |
| `tag`              | `string \| undefined`        | Tag, if provided                                |

Each entry in `burnAddresses` is an `InvoiceBatchBurnAddress`:

| Field          | Type     | Description                                |
| -------------- | -------- | ------------------------------------------ |
| `subId`        | `number` | Sub-index within the invoice (0-based)     |
| `burnAddress`  | `string` | Derived burn address                       |
| `secret`       | `string` | Secret used in burn address derivation     |
| `tweak`        | `string` | Tweak value for address uniqueness         |

For a single invoice, `burnAddresses` contains one entry. For a batch invoice, it contains exactly 10.

## Submit Invoice

After preparing the invoice, sign the `signatureMessage` with the recipient's wallet and submit the invoice to the ICP storage canister.

```typescript
import { submitInvoice } from "zerc20-client-sdk";
import { hexToBytes } from "viem";

// Sign the message from the preparation step
const signature = await walletClient.signMessage({
  message: artifacts.signatureMessage,
});

// Submit to the canister (convert hex signature to Uint8Array)
await submitInvoice(
  client,                       // StealthCanisterClient
  artifacts.invoiceId,          // hex-encoded invoice ID
  hexToBytes(signature),        // Uint8Array signature
  artifacts.tag,                // optional tag
);
```

**Signature:**

```typescript
function submitInvoice(
  client: StealthCanisterClient,
  invoiceIdHex: string,
  signature: Uint8Array,
  tag?: string,
): Promise<void>;
```

The canister verifies the signature against the recipient address embedded in the invoice before accepting it.

## List Invoices

Retrieve invoice IDs owned by a specific address. Optionally filter by chain ID or tag.

```typescript
import { listInvoices } from "zerc20-client-sdk";

const invoiceIds = await listInvoices(
  client,          // StealthCanisterClient
  ownerAddress,    // EVM address that created the invoices
  chainId,         // optional, filter by recipient chain ID
  tag,             // optional, filter by tag
);

// invoiceIds: string[] — hex-encoded invoice IDs
```

**Signature:**

```typescript
function listInvoices(
  client: StealthCanisterClient,
  ownerAddress: string,
  chainId?: number | bigint,
  tag?: string,
): Promise<string[]>;
```

Returns an array of invoice IDs as hex strings. Use these IDs to look up or share specific invoices.

## Checking Invoice Type

The SDK provides helpers to determine whether an invoice ID represents a single or batch invoice:

```typescript
import { isSingleInvoiceHex, isSingleInvoiceBytes } from "zerc20-client-sdk";

// From hex string
const isSingle = isSingleInvoiceHex(invoiceId);

// From Uint8Array
const isSingle = isSingleInvoiceBytes(invoiceBytes);
```

This is useful when deciding which proof mode to use for redemption -- single invoices use Groth16, batch invoices use Nova.

## Complete Example

End-to-end flow: prepare an invoice, sign it, submit it, and list all invoices.

```typescript
import {
  prepareInvoiceIssue,
  submitInvoice,
  listInvoices,
} from "zerc20-client-sdk";
import { hexToBytes } from "viem";

// 1. Prepare the invoice
const artifacts = await prepareInvoiceIssue({
  client,
  seedHex: "0xabcdef1234567890...",
  recipientAddress: "0xRecipient...",
  recipientChainId: 8453, // Base
  isBatch: false,
});

console.log("Invoice ID:", artifacts.invoiceId);
console.log("Burn address:", artifacts.burnAddresses[0].burnAddress);

// 2. Sign the signature message with the recipient's wallet
const signature = await walletClient.signMessage({
  message: artifacts.signatureMessage,
});

// 3. Submit the signed invoice to the ICP storage canister
await submitInvoice(
  client,
  artifacts.invoiceId,
  hexToBytes(signature),  // convert hex string to Uint8Array
  artifacts.tag,
);

console.log("Invoice submitted successfully");

// 4. List all invoices for this address
const invoiceIds = await listInvoices(
  client,
  "0xRecipient...",
);

console.log("All invoices:", invoiceIds);
```

Once submitted, the sender can retrieve the invoice to learn the burn address and execute a [Private Send](private-send.md). For more on setting up the SDK client, see the [SDK Quick Start](quickstart.md).
