# Receiving

Receiving zERC20 tokens is a two-phase process:

1. **Authorize + Scan**: Authenticate with the ICP stealth storage canister, decrypt your VetKey, and scan for announcements addressed to you.
2. **Redeem**: Collect on-chain context for eligible transfers, generate a zero-knowledge proof, and submit the redeem transaction to mint tokens.

## Step 1: Create Authorization Payload

Build a time-limited authorization payload that will be signed by the recipient's wallet. The `ttlSeconds` parameter controls how long the authorization remains valid.

```typescript
import { createAuthorizationPayload } from "zerc20-client-sdk";

const payload = await createAuthorizationPayload(
  client,          // StealthCanisterClient
  address,         // recipient EVM address
  ttlSeconds,      // optional, defaults to a sensible value
);
```

**Signature:**

```typescript
function createAuthorizationPayload(
  client: StealthCanisterClient,
  address: string,
  ttlSeconds?: number,
): Promise<AuthorizationPayload>;
```

**AuthorizationPayload:**

| Field              | Type     | Description                                            |
| ------------------ | -------- | ------------------------------------------------------ |
| `message`          | `string` | Human-readable message to display in the wallet prompt |
| `canonicalMessage` | `Uint8Array` | Canonical form used for on-canister verification       |
| `expiryNs`        | `bigint` | Expiry timestamp in nanoseconds                        |
| `nonce`            | `bigint` | Random nonce to prevent replay attacks                 |
| `transport`        | `object` | Ephemeral transport key pair for VetKey decryption     |

## Step 2: Sign the Authorization Message

Sign the authorization message using EIP-191 personal sign. Any standard wallet client works.

```typescript
const signature = await walletClient.signMessage({
  message: payload.message,
});
```

The wallet will display `payload.message` to the user for approval. The resulting signature is used in the next step to prove ownership of the address.

## Step 3: Request VetKey

Submit the signed authorization to the ICP canister to retrieve your VetKey. The VetKey is encrypted in transit using the ephemeral transport key pair from Step 1 and decrypted locally -- the canister never sees the plaintext key.

```typescript
import { requestVetKey } from "zerc20-client-sdk";

const vetKey = await requestVetKey(
  client,     // StealthCanisterClient
  address,    // recipient EVM address
  payload,    // AuthorizationPayload from Step 1
  signature,  // Uint8Array, raw signature bytes
);
```

**Signature:**

```typescript
function requestVetKey(
  client: StealthCanisterClient,
  address: string,
  payload: AuthorizationPayload,
  signature: Uint8Array,
): Promise<VetKey>;
```

The returned `VetKey` is used to decrypt announcements stored on ICP.

## Step 4: Scan for Announcements

Scan the stealth storage canister for announcements that can be decrypted with your VetKey. Each announcement corresponds to a private transfer sent to you.

```typescript
import { scanReceivings } from "zerc20-client-sdk";

const announcements = await scanReceivings({
  client,             // StealthCanisterClient
  vetKey,             // VetKey from Step 3
  pageSize: 100,      // optional, default: 100
  startAfter: undefined, // optional, resume from a previous scan
  tag: undefined,     // optional, filter by tag
});
```

**Signature:**

```typescript
function scanReceivings(
  params: ScanReceivingsParams,
): Promise<ScannedAnnouncement[]>;
```

**ScanReceivingsParams:**

| Field        | Type                     | Required | Description                                      |
| ------------ | ------------------------ | -------- | ------------------------------------------------ |
| `client`     | `StealthCanisterClient`  | Yes      | ICP canister client                              |
| `vetKey`     | `VetKey`                 | Yes      | Decryption key from Step 3                       |
| `pageSize`   | `number`                 | No       | Number of announcements per page (default: 100)  |
| `startAfter` | `bigint \| undefined`    | No       | Announcement ID to resume scanning after         |
| `tag`        | `string \| undefined`    | No       | Filter announcements by tag                      |

**ScannedAnnouncement:**

| Field              | Type     | Description                                   |
| ------------------ | -------- | --------------------------------------------- |
| `id`               | `bigint` | Unique announcement identifier                |
| `burnAddress`      | `string` | Truncated burn address (on-chain destination) |
| `fullBurnAddress`  | `string` | Full burn address before truncation           |
| `createdAtNs`      | `bigint` | Creation timestamp in nanoseconds             |
| `recipientChainId` | `bigint` | Chain ID where the recipient will redeem      |

## Step 5: Collect Redeem Context

For each scanned announcement, collect the on-chain context needed to generate a redemption proof. This queries the indexer and contracts to determine which transfers are eligible for redemption.

```typescript
import { collectRedeemContext, createVerifierReader } from "zerc20-client-sdk";

// Create a verifier contract reader from an EvmReadProvider
const verifierContract = createVerifierReader(readProvider, entry.verifierAddress);

const redeemContext = await collectRedeemContext({
  burn,               // BurnArtifacts derived from the scanned announcement
  tokens,             // token configuration (TokenEntry[])
  hub,                // HubEntry from normalizeTokens
  verifierContract,   // ReadableVerifierContract from createVerifierReader
  indexerUrl,         // indexer endpoint URL
  indexerFetchLimit,  // optional, max events per indexer request
  eventBlockSpan,     // optional, block range per scan
});
```

**Creating the Verifier Contract:**

Use `createVerifierReader()` to build a `ReadableVerifierContract` from any `EvmReadProvider`. This is the recommended approach -- it works with viem, ethers.js, or any provider that implements the `EvmReadProvider` interface:

```typescript
import { createVerifierReader } from "zerc20-client-sdk";

const verifierContract = createVerifierReader(readProvider, entry.verifierAddress);
```

**Signature:**

```typescript
function collectRedeemContext(
  params: RedeemContextParams,
): Promise<RedeemContext>;
```

**RedeemContextParams:**

| Field               | Type                        | Required | Description                            |
| ------------------- | --------------------------- | -------- | -------------------------------------- |
| `burn`              | `BurnArtifacts`             | Yes      | Burn artifacts for the announcement    |
| `tokens`            | `TokenEntry[]`              | Yes      | Token configuration                    |
| `hub`               | `HubEntry`                  | Yes      | Hub contract address or config         |
| `verifierContract`  | `ReadableVerifierContract`  | Yes      | Verifier contract reader               |
| `indexerUrl`        | `string`                    | Yes      | Indexer HTTP endpoint                  |
| `indexerFetchLimit` | `number`                    | No       | Max events per indexer request         |
| `eventBlockSpan`    | `bigint \| number`          | No       | Block range per event scan             |

**RedeemContext:**

| Field                | Type                | Description                                              |
| -------------------- | ------------------- | -------------------------------------------------------- |
| `token`              | `TokenInfo`         | Resolved token metadata                                  |
| `aggregationState`   | `AggregationState`  | Current Hub aggregation snapshot                         |
| `events`             | `object`            | Contains `eligible` and `ineligible` event arrays        |
| `globalProofs`       | `GlobalProof[]`     | Global Merkle proofs for eligible events                 |
| `eligibleProofs`     | `EligibleProof[]`   | Per-event proofs ready for ZKP generation                |
| `totalEligibleValue` | `bigint`            | Sum of values that can be redeemed now                   |
| `totalPendingValue`  | `bigint`            | Sum of values not yet included in a proven root          |
| `totalIndexedValue`  | `bigint`            | Sum of all indexed values for this burn address          |
| `totalTeleported`    | `bigint`            | Amount already teleported (minted) to the recipient      |
| `chains`             | `ChainBreakdown[]`  | Per-chain breakdown of eligible and pending values       |

- **Eligible events**: Transfers whose Merkle roots have been proven on-chain and aggregated by the Hub. These can be redeemed immediately.
- **Ineligible events**: Transfers that are indexed but whose roots are not yet proven or aggregated. These will become eligible once the indexer and cross-chain job catch up.

## Step 6: Prepare and Submit Redeem Transaction

The SDK provides a two-step prepare/submit pattern for redeeming. `prepareRedeemTransaction()` generates the zero-knowledge proof and assembles the transaction data, and `submitRedeemTransaction()` signs and submits it on-chain.

```typescript
import {
  prepareRedeemTransaction,
  submitRedeemTransaction,
} from "zerc20-client-sdk";

// Prepare: generates proof and builds the transaction object
const redeemTx = await prepareRedeemTransaction({
  redeemContext,
  burn,                       // BurnArtifacts
  teleportProofClient: sdk.teleportProofs,
  decider: sdk.decider,       // optional, required for batch proofs
});

// Submit: signs and sends the transaction on-chain
const { transactionHash } = await submitRedeemTransaction({
  writeProvider,              // EvmWriteProvider
  tx: redeemTx,               // RedeemTransaction from prepare step
  readProvider,               // optional: for receipt polling
  feeOverrides,               // optional: gas-price overrides
});
```

The SDK automatically selects the proof mode based on the number of eligible events:

- **1 eligible event** -- uses Groth16 single proof via `Verifier.singleTeleport()`
- **Multiple eligible events** -- uses Nova batch proof via `Verifier.teleport()` (requires a Decider)

### Optional: Relayer-assisted redeem

If you want a relay node to submit the redeem transaction on the user's behalf, attach a
`RelayerFeeAuthorization` when preparing the transaction. This lets the user cap the fee the
relayer may keep while avoiding the need to hold native gas on the destination chain.

```typescript
import {
  prepareRedeemTransaction,
  submitRelayTeleport,
  type RelayerFeeAuthorization,
} from "zerc20-client-sdk";

const feeAuth: RelayerFeeAuthorization = {
  relayerFee: 50_000n,
  maxFee: 60_000n,
  deadline: 1_700_000_000n,
  signature: "0x...",
};

const redeemTx = await prepareRedeemTransaction({
  redeemContext,
  burn,
  teleportProofClient: sdk.proofs,
  decider: sdk.decider,
  relayerFeeAuth: feeAuth,
});

await submitRelayTeleport("https://relay.example", {
  isSingle: redeemTx.mode === "single",
  isGlobal: true,
  rootHint: redeemContext.aggregationState.latestAggSeq,
  chainId: redeemTx.args[2].chainId,
  recipient: redeemTx.args[2].recipient,
  tweak: redeemTx.args[2].tweak,
  proof: redeemTx.args[3] as `0x${string}`,
  relayerFee: feeAuth.relayerFee,
  maxFee: feeAuth.maxFee,
  deadline: feeAuth.deadline,
  signature: feeAuth.signature,
});
```

Practical notes:

- `relayerFeeAuth` is optional. Omit it for the normal direct-wallet redeem flow.
- Relay nodes typically expose a fee quote endpoint first; use that quote to build the authorization.
- The relay returns a transaction hash after submission. This indicates broadcast, not final mined success.

### prepareRedeemTransaction

```typescript
function prepareRedeemTransaction(
  params: PrepareRedeemTransactionParams,
): Promise<RedeemTransaction>;
```

**PrepareRedeemTransactionParams:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `redeemContext` | `RedeemContext` | Yes | Context from `collectRedeemContext()` |
| `burn` | `BurnArtifacts` | Yes | Burn artifacts for the announcement |
| `teleportProofClient` | `TeleportProofClient` | Yes | Proof generator (from `sdk.teleportProofs`) |
| `decider` | `HttpDeciderClient` | No | Required for batch proofs; omit for single-only |
| `relayerFeeAuth` | `RelayerFeeAuthorization` | No | Required only when preparing a relayer-assisted redeem |

**RedeemTransaction:**

| Field | Type | Description |
|-------|------|-------------|
| `mode` | `"single" \| "batch"` | Which proof mode was used |
| `address` | `Hex` | Verifier contract address |
| `abi` | `object` | Verifier ABI |
| `functionName` | `"singleTeleport" \| "teleport"` | Contract function to call |
| `args` | `readonly [...]` | Encoded arguments for the contract call |

### submitRedeemTransaction

```typescript
function submitRedeemTransaction(
  params: SubmitRedeemTransactionParams,
): Promise<{ transactionHash: Hex }>;
```

**SubmitRedeemTransactionParams:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `writeProvider` | `EvmWriteProvider` | Yes | Wallet provider to sign and send |
| `tx` | `RedeemTransaction` | Yes | Transaction from `prepareRedeemTransaction()` |
| `feeOverrides` | `FeeOverrides` | No | Optional gas-price overrides |
| `readProvider` | `EvmReadProvider` | No | Provider for receipt polling |

### Batch Redeem with Pre-built Proof

If you already have a Decider proof (e.g., from an external service), use `buildBatchRedeemTransaction()` to assemble the transaction directly:

```typescript
import { buildBatchRedeemTransaction } from "zerc20-client-sdk";

const redeemTx = buildBatchRedeemTransaction({
  redeemContext,
  burn,
  deciderProof: proofBytes,  // Uint8Array from Decider
});
```

## Status Checking

For a lighter-weight check that skips proof collection and generation, use `getAnnouncementStatus`. This is useful for displaying balances or polling for readiness without the overhead of `collectRedeemContext`.

```typescript
import { getAnnouncementStatus } from "zerc20-client-sdk";

const status = await getAnnouncementStatus({
  burn,
  tokens,
  hub,
  verifierContract,
  indexerUrl,
});
```

**Signature:**

```typescript
function getAnnouncementStatus(
  params: AnnouncementStatusParams,
): Promise<AnnouncementStatus>;
```

**AnnouncementStatus:**

| Field                | Type     | Description                                          |
| -------------------- | -------- | ---------------------------------------------------- |
| `totalEligibleValue` | `bigint` | Sum of values that can be redeemed now               |
| `totalPendingValue`  | `bigint` | Sum of values not yet included in a proven root      |
| `totalIndexedValue`  | `bigint` | Sum of all indexed values for this burn address      |
| `totalTeleported`    | `bigint` | Amount already teleported (minted) to the recipient  |
