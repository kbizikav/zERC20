# API Reference

Complete reference of all public exports from `zerc20-client-sdk`.

## Core

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `createSdk` | `(options?: Zerc20SdkOptions)` | `Zerc20Sdk` | Create SDK instance |
| `Zerc20Sdk` | class | - | Main SDK, bundles WASM, proof, decider, stealth |
| `Zerc20SdkOptions` | interface | - | Options: `wasm?`, `teleportProofs?`, `decider?`, `stealth?` |

## ICP / Stealth

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `StealthCanisterClient` | class | - | ICP canister client for announcements, invoices, keys |
| `StealthClientFactory` | class | - | Factory for creating StealthCanisterClient instances |
| `StealthClientConfig` | interface | - | Config: `agent`, `storageCanisterId`, `keyManagerCanisterId` |
| `createAuthorizationPayload` | `(client, address, ttlSeconds?)` | `Promise<AuthorizationPayload>` | Create auth payload for VetKey request |
| `requestVetKey` | `(client, address, payload, signature)` | `Promise<VetKey>` | Request and decrypt VetKey |
| `scanReceivings` | `(params: ScanReceivingsParams)` | `Promise<ScannedAnnouncement[]>` | Scan and decrypt announcements |

## Private Send

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `preparePrivateSend` | `(params: PreparePrivateSendParams)` | `Promise<PreparedPrivateSend>` | Derive burn address and encrypt announcement |
| `submitPrivateSendAnnouncement` | `(params: SubmitPrivateSendParams)` | `Promise<PrivateSendResult>` | Submit announcement to storage canister |

## Invoice

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `prepareInvoiceIssue` | `(params: InvoiceIssueParams)` | `Promise<InvoiceIssueArtifacts>` | Generate invoice with burn addresses |
| `submitInvoice` | `(client, invoiceIdHex, signature, tag?)` | `Promise<void>` | Submit signed invoice |
| `listInvoices` | `(client, ownerAddress, chainId?, tag?)` | `Promise<string[]>` | List invoice IDs |

## Liquidity

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `wrapWithLiquidityManager` | `(params: WrapWithLiquidityManagerParams)` | `Promise<LiquidityActionResult>` | Wrap underlying to zERC20 |
| `unwrapWithLiquidityManager` | `(params: LocalUnwrapParams)` | `Promise<LiquidityActionResult>` | Unwrap zERC20 to underlying |
| `quoteLocalUnwrap` | `(params)` | `Promise<LocalUnwrapQuote>` | Quote unwrap fee |
| `buildCrossUnwrapQuote` | `(params)` | `Promise<CrossUnwrapQuote>` | Quote cross-chain unwrap |
| `sendCrossUnwrap` | `(params)` | `Promise<LiquidityActionResult>` | Execute cross-chain unwrap |
| `fetchLiquidityManagerBalances` | `(params)` | `Promise<LiquidityBalances>` | Get token balances and decimals |

## Receive

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `collectRedeemContext` | `(params: RedeemContextParams)` | `Promise<RedeemContext>` | Collect eligible events + proofs for redeem |
| `getAnnouncementStatus` | `(params: AnnouncementStatusParams)` | `Promise<AnnouncementStatus>` | Lightweight status check |

## Proofs

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `TeleportProofClient` | class | - | High-level proof client |
| `createTeleportProofClient` | `(options?)` | `TeleportProofClient` | Create proof client |
| `HttpDeciderClient` | class | - | Decider service HTTP client |

## WASM

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `WasmRuntime` | class | - | WASM lifecycle manager |
| `getSeedMessage` | `()` | `Promise<string>` | Get message to sign for seed derivation |
| `derivePaymentAdvice` | `(seedHex, paymentAdviceIdHex, chainId, address)` | `Promise<SecretAndTweak>` | Derive payment advice |
| `buildFullBurnAddress` | `(chainId, address, secret, tweak)` | `Promise<BurnArtifacts>` | Build burn address |
| `decodeFullBurnAddress` | `(fullBurnAddressHex)` | `Promise<BurnArtifacts>` | Decode burn address |

## Registry

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `loadTokens` | `(compressed, options?)` | `Promise<NormalizedTokens>` | Load tokens from compressed (Base64 gzip) string |
| `normalizeTokens` | `(file: TokensFile)` | `NormalizedTokens` | Normalize raw token config |
| `findTokenByChain` | `(tokens, chainId)` | `TokenEntry` | Find token by chain ID |
| `createProviderForToken` | `(entry)` | `PublicClient` | Create RPC provider |
| `clearTokensCache` | `()` | `void` | Clear token cache |

## Contracts

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `getZerc20Contract` | `(address, client)` | contract instance | Get zERC20 contract |
| `getVerifierContract` | `(address, client)` | contract instance | Get Verifier contract |
| `getHubContract` | `(address, client)` | contract instance | Get Hub contract |
| `getLiquidityManagerContract` | `(address, client)` | contract instance | Get LiquidityManager contract |
| `getAdaptorContract` | `(address, client)` | contract instance | Get Adaptor contract |

## Types

Key exported types and interfaces:

- `SecretAndTweak` -- Secret and tweak pair derived from payment advice.
- `GeneralRecipient` -- Recipient descriptor used across send and invoice flows.
- `BurnArtifacts` -- Artifacts produced when building or decoding a burn address.
- `PrivateSendPreparation` -- Intermediate preparation data for a private send.
- `PreparedPrivateSend` -- Fully prepared private send, ready for on-chain transfer and announcement submission.
- `PrivateSendResult` -- Result after submitting a private send announcement.
- `InvoiceIssueArtifacts` -- Artifacts from invoice preparation, including burn addresses and encrypted data.
- `InvoiceBatchBurnAddress` -- A single burn address within a batch invoice.
- `ScannedAnnouncement` -- A decrypted announcement discovered during scanning.
- `AggregationTreeState` -- State of the cross-chain aggregation Merkle tree.
- `GlobalTeleportProof` -- A proof valid against the global (aggregated) tree root.
- `IndexedEvent` -- An on-chain transfer event as indexed by the indexer.
- `SingleTeleportArtifacts` -- Artifacts for a single Groth16 teleport proof.
- `SingleTeleportParams` -- Parameters for generating a single teleport proof.
- `NovaProverInput` -- Input to the Nova batch prover.
- `NovaProverOutput` -- Output from the Nova batch prover.
- `BatchTeleportArtifacts` -- Artifacts for a batch (Nova + Groth16) teleport proof.
- `BatchTeleportParams` -- Parameters for generating a batch teleport proof.
- `RedeemContext` -- Full context needed to execute a redeem (events, proofs, chain data).
- `RedeemChainContext` -- Per-chain subset of redeem context.
- `AnnouncementStatus` -- Lightweight status of an announcement (pending, redeemable, redeemed).
- `EventsWithEligibility` -- Events annotated with their eligibility for redemption.
- `TokenEntry` -- A single token's deployment configuration.
- `HubEntry` -- Hub contract deployment configuration.
- `TokensFile` -- Raw deserialized token configuration file.
- `NormalizedTokens` -- Normalized token configuration with parsed `bigint` fields.

## Constants

| Name | Value | Description |
|------|-------|-------------|
| `AGGREGATION_TREE_HEIGHT` | `6` | Aggregation tree levels |
| `TRANSFER_TREE_HEIGHT` | `40` | Per-chain transfer tree levels |
| `GLOBAL_TRANSFER_TREE_HEIGHT` | `46` | Global tree (40 + 6) |
| `NUM_BATCH_INVOICES` | `10` | Burn addresses per batch invoice |
| `DEFAULT_INDEXER_FETCH_LIMIT` | `20` | Max events per indexer request |
| `DEFAULT_EVENT_BLOCK_SPAN` | `5000n` | Block span for aggregation queries |
| `DEFAULT_DECIDER_TIMEOUT_MS` | `300000` | Decider job timeout (5 min) |
| `DEFAULT_DECIDER_POLL_INTERVAL_MS` | `1000` | Decider poll interval |
| `DEFAULT_ZSTORAGE_TAG` | `"v1"` | Default IC storage tag |
