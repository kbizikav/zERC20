# Receiving

Receiving zERC20 tokens is a two-phase process:

1. **Authorize + Scan**: Authenticate with the ICP stealth storage canister, decrypt your VetKey, and scan for announcements addressed to you.
2. **Redeem**: Collect on-chain context for eligible transfers, generate a zero-knowledge proof, and call `teleport` on the Verifier contract to mint tokens.

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
import { collectRedeemContext } from "zerc20-client-sdk";

const redeemContext = await collectRedeemContext({
  burn,               // BurnArtifacts derived from the scanned announcement
  tokens,             // token configuration
  hub,                // Hub contract address or config
  verifierContract,   // Verifier contract instance
  indexerUrl,         // indexer endpoint URL
  indexerFetchLimit,  // optional, max events per indexer request
  eventBlockSpan,     // optional, block range per scan
});
```

**Signature:**

```typescript
function collectRedeemContext(
  params: RedeemContextParams,
): Promise<RedeemContext>;
```

**RedeemContextParams:**

| Field               | Type             | Required | Description                            |
| ------------------- | ---------------- | -------- | -------------------------------------- |
| `burn`              | `BurnArtifacts`  | Yes      | Burn artifacts for the announcement    |
| `tokens`            | `TokenConfig`    | Yes      | Token configuration                    |
| `hub`               | `HubConfig`      | Yes      | Hub contract address or config         |
| `verifierContract`  | `Contract`       | Yes      | Verifier contract instance             |
| `indexerUrl`        | `string`         | Yes      | Indexer HTTP endpoint                  |
| `indexerFetchLimit` | `number`         | No       | Max events per indexer request         |
| `eventBlockSpan`    | `bigint \| number` | No     | Block range per event scan             |

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

## Step 6: Generate Proof and Teleport

Use the eligible events and global proofs from `RedeemContext` to generate a zero-knowledge proof and submit it on-chain. See [Proof Generation](proof-generation.md) for the full API reference.

Two redemption paths are available:

### Single Teleport

Redeem a single eligible event with a Groth16 proof:

```typescript
const proof = await createSingleTeleportProof(/* ... */);
// Submit to Verifier.singleTeleport()
```

### Batch Teleport

Redeem multiple eligible events at once using a Nova batch proof:

```typescript
const proof = await createBatchTeleportProof(/* ... */);
// Submit to Verifier.teleport()
```

Both functions accept the `eligibleProofs` and `globalProofs` from `RedeemContext` as inputs.

## Status Checking

For a lighter-weight check that skips proof collection and generation, use `getAnnouncementStatus`. This is useful for displaying balances or polling for readiness without the overhead of `collectRedeemContext`.

```typescript
import { getAnnouncementStatus } from "zerc20-client-sdk";

const status = await getAnnouncementStatus({
  // AnnouncementStatusParams
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
