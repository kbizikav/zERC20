# API Reference

Complete reference of all public exports from `zerc20-client-sdk`.

## Core

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `createSdk` | `(options?: Zerc20SdkOptions)` | `Zerc20Sdk` | Create SDK instance |
| `Zerc20Sdk` | class | - | Main SDK, bundles WASM, proof, decider, stealth |
| `Zerc20SdkOptions` | interface | - | Options: `wasm?`, `proofs?`, `decider?`, `stealth?` |

## EVM Providers

Library-agnostic interfaces for EVM interaction. viem's `PublicClient` and `WalletClient` satisfy these structurally -- no adapter required.

| Name | Type | Description |
|------|------|-------------|
| `EvmReadProvider` | interface | Read-only provider: `readContract`, optional `getBalance`, `estimateFeesPerGas`, `getGasPrice`, `waitForTransactionReceipt`, `getTransaction`, `getTransactionReceipt` |
| `EvmWriteProvider` | interface | Write provider: `writeContract`, optional `account`, `chain` |
| `Hex` | type | Hex-encoded string with `0x` prefix (`\`0x${string}\``) |

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
| `submitPrivateSendTransfer` | `(params: SubmitPrivateSendTransferParams)` | `Promise<{ transactionHash: Hex }>` | Execute ERC-20 transfer to burn address |

## Invoice

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `prepareInvoiceIssue` | `(params: InvoiceIssueParams)` | `Promise<InvoiceIssueArtifacts>` | Generate invoice with burn addresses |
| `submitInvoice` | `(client, invoiceIdHex, signature, tag?)` | `Promise<void>` | Submit signed invoice |
| `listInvoices` | `(client, ownerAddress, chainId?, tag?)` | `Promise<string[]>` | List invoice IDs |
| `isSingleInvoiceHex` | `(invoiceIdHex: string)` | `boolean` | Check if an invoice ID is single (not batch) |
| `isSingleInvoiceBytes` | `(invoiceBytes: Uint8Array)` | `boolean` | Check if invoice bytes represent a single invoice |

## Liquidity

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `wrapWithLiquidityManager` | `(params: WrapWithLiquidityManagerParams)` | `Promise<LiquidityActionResult>` | Wrap underlying to zERC20 |
| `unwrapWithLiquidityManager` | `(params: LocalUnwrapParams)` | `Promise<LiquidityActionResult>` | Unwrap zERC20 to underlying |
| `quoteLocalUnwrap` | `(params)` | `Promise<LocalUnwrapQuote>` | Quote unwrap fee |
| `buildCrossUnwrapQuote` | `(params)` | `Promise<CrossUnwrapQuote>` | Quote cross-chain unwrap |
| `sendCrossUnwrap` | `(params)` | `Promise<LiquidityActionResult>` | Execute cross-chain unwrap |
| `fetchLiquidityManagerBalances` | `(params)` | `Promise<LiquidityBalances>` | Get token balances and decimals |
| `fetchAdaptorBalances` | `(params)` | `Promise<AdaptorBalances>` | Get stuck fund balances from an adaptor contract |
| `hasStuckFunds` | `(params)` | `Promise<boolean>` | Check if any funds are stuck in an adaptor |
| `withdrawFromAdaptor` | `(params: AdaptorWithdrawParams)` | `Promise<{ transactionHash: string }>` | Withdraw stuck funds from an adaptor |
| `NATIVE_TOKEN_ADDRESS` | constant | `` `0x${string}` `` | Native token sentinel address (`0xEeee...`) |
| `removeDust` | `(amount, conversionRate)` | `bigint` | Remove dust from amount based on decimal conversion |

## Receive

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `collectRedeemContext` | `(params: RedeemContextParams)` | `Promise<RedeemContext>` | Collect eligible events + proofs for redeem |
| `createVerifierReader` | `(provider: EvmReadProvider, address: string)` | `ReadableVerifierContract` | Create verifier contract reader from a provider |
| `prepareRedeemTransaction` | `(params: PrepareRedeemTransactionParams)` | `Promise<RedeemTransaction>` | Generate proof and assemble redeem transaction |
| `buildBatchRedeemTransaction` | `(params)` | `RedeemTransaction` | Build batch redeem from pre-existing Decider proof |
| `submitRedeemTransaction` | `(params: SubmitRedeemTransactionParams)` | `Promise<{ transactionHash: Hex }>` | Sign and submit redeem transaction on-chain |
| `getAnnouncementStatus` | `(params: AnnouncementStatusParams)` | `Promise<AnnouncementStatus>` | Lightweight status check |

## Chain Metadata

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `getChainMetadata` | `(chainId: number \| bigint)` | `ChainMetadata \| undefined` | Get metadata for a chain ID |
| `getChainDisplayName` | `(chainId: number \| bigint)` | `string` | Human-readable chain name (e.g., "Ethereum", "Arbitrum") |
| `getChainShortName` | `(chainId: number \| bigint)` | `string \| undefined` | Short chain label (e.g., "ETH", "ARB") |
| `getExplorerTxUrl` | `(chainId: number \| bigint, txHash: string)` | `string \| undefined` | Block explorer URL for a transaction |
| `resolveChainId` | `(name: string)` | `number \| undefined` | Resolve chain ID from name or alias |
| `resolveNetworkDisplayName` | `(label: string)` | `string` | Resolve human-readable name from chain label |

## LayerZero Scan

Cross-chain message tracking via the LayerZero Scan API. See [LayerZero Scan](layerzero-scan.md) for full documentation.

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `fetchWalletStatus` | `(params: FetchWalletStatusParams)` | `Promise<WalletStatusResult>` | Fetch and decode all LZ messages for a wallet |
| `fetchWalletMessages` | `(config, address, params?)` | `Promise<ScanMessagesResponse>` | Raw LZ Scan API: wallet messages |
| `fetchTxMessages` | `(config, txHash)` | `Promise<ScanMessagesResponse>` | Raw LZ Scan API: messages for a transaction |
| `getWalletMessagesUrl` | `(config, address)` | `string` | Build LZ Scan explorer URL |
| `tryDecodeBridgeRequest` | `(composeMsg: Hex)` | `BridgeRequestSummary \| null` | Decode a compose message as a bridge request |
| `decodeSendSummary` | `(data: Hex)` | `SendPayloadSummary` | Decode an OFT send payload |
| `fetchOftSentAmount` | `(provider, txHash)` | `Promise<bigint \| undefined>` | Read OFTSent amount from transaction logs |
| `decorateSendSummary` | `(summary, token?, fetchMetadata?)` | decorated summary | Enrich send summary with token metadata |
| `endpointChain` | `(endpoint, direction)` | `string` | Chain name from LZ endpoint |
| `destinationTx` | `(message)` | `string \| undefined` | Extract destination tx hash |
| `summarizeBlock` | `(message, direction)` | `string` | Summarize block info |
| `formatPathway` | `(message)` | `string` | Format src → dst pathway |
| `isMessageForTokens` | `(message, tokens)` | `boolean` | Check if message involves given tokens |
| `findTokenForMessage` | `(message, tokens)` | `TokenEntry \| undefined` | Find matching token for a message |

## Onchain

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `readTokenBalance` | `(provider, tokenAddress, account)` | `Promise<bigint>` | Read ERC-20 balance via provider |
| `readTokenDecimals` | `(provider, tokenAddress)` | `Promise<number>` | Read ERC-20 decimals via provider |
| `readDecimalConversionRate` | `(provider, tokenAddress)` | `Promise<bigint>` | Read zERC20 decimal conversion rate |
| `decodeSendPayload` | `(data: Hex)` | `DecodedSendPayload` | Decode OFT send() calldata |
| `extractOftSentAmount` | `(logs)` | `bigint \| undefined` | Extract OFTSent amount from tx logs |
| `decodeBridgeRequest` | `(composeMsg: Hex)` | `DecodedBridgeRequest \| null` | Decode a bridge compose message |

## Proofs

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `ProofService` | class | - | ZK proof service for single and batch redemption |
| `ProofServiceOptions` | interface | - | Options for worker offload and proof execution |
| `HttpDeciderClient` | class | - | Decider service HTTP client |

## WASM

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `WasmRuntime` | class | - | WASM lifecycle manager |
| `getSeedMessage` | `()` | `Promise<string>` | Get message to sign for seed derivation |
| `deriveSeed` | `(signMessage)` | `Promise<string>` | Derive seed using a sign callback |
| `derivePaymentAdvice` | `(seedHex, paymentAdviceIdHex, chainId, address)` | `Promise<SecretAndTweak>` | Derive payment advice |
| `buildFullBurnAddress` | `(chainId, address, secret, tweak)` | `Promise<BurnArtifacts>` | Build burn address |
| `decodeFullBurnAddress` | `(fullBurnAddressHex)` | `Promise<BurnArtifacts>` | Decode burn address |

## Registry

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `normalizeTokens` | `(file: TokensFile)` | `NormalizedTokens` | Normalize raw token config |
| `normalizeTokensWithOverrides` | `(file: TokensFile, overrides?: RpcOverrides)` | `NormalizedTokens` | Normalize with RPC URL overrides |
| `findTokenByChain` | `(tokens, chainId)` | `TokenEntry` | Find token by chain ID |
| `TokensCacheManager` | class | - | Cache manager for compressed token loading |

## Contract Artifacts

The SDK exports ABI artifacts for on-chain contracts. These can be used with any EVM library for direct contract interaction.

| Name | Description |
|------|-------------|
| `Zerc20Artifact` | zERC20 token contract ABI |
| `VerifierArtifact` | Verifier contract ABI |
| `HubArtifact` | Hub contract ABI |
| `LiquidityManagerArtifact` | LiquidityManager contract ABI |
| `AdaptorArtifact` | Adaptor contract ABI |

## Utilities

| Name | Signature | Returns | Description |
|------|-----------|---------|-------------|
| `buildFeeOverrides` | `(provider: EvmReadProvider)` | `Promise<FeeOverrides>` | Build gas fee overrides from provider |
| `isHex` | `(value: unknown)` | `boolean` | Check if value is a valid 0x-prefixed hex string |
| `keccak256` | `(input)` | `Hex` | Compute keccak256 hash |

## Types

Key exported types and interfaces:

- `EvmReadProvider` -- Library-agnostic read-only EVM provider interface.
- `EvmWriteProvider` -- Library-agnostic write EVM provider interface.
- `Hex` -- Hex-encoded string type (`\`0x${string}\``).
- `SecretAndTweak` -- Secret and tweak pair derived from payment advice.
- `GeneralRecipient` -- Recipient descriptor used across send and invoice flows.
- `BurnArtifacts` -- Artifacts produced when building or decoding a burn address.
- `PreparedPrivateSend` -- Fully prepared private send, ready for on-chain transfer and announcement submission.
- `PrivateSendResult` -- Result after submitting a private send announcement.
- `SubmitPrivateSendTransferParams` -- Parameters for the ERC-20 transfer step.
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
- `RedeemContext` -- Full context needed to execute a redeem (events, proofs, chain data).
- `RedeemTransaction` -- Prepared transaction data for on-chain redeem submission.
- `ReadableVerifierContract` -- Verifier contract interface for `readContract`-based reads.
- `OnChainGeneralRecipient` -- On-chain representation of a general recipient.
- `AnnouncementStatus` -- Lightweight status of an announcement (pending, redeemable, redeemed).
- `EventsWithEligibility` -- Events annotated with their eligibility for redemption.
- `TokenEntry` -- A single token's deployment configuration.
- `HubEntry` -- Hub contract deployment configuration.
- `TokensFile` -- Raw deserialized token configuration file.
- `NormalizedTokens` -- Normalized token configuration with parsed `bigint` fields.
- `RpcOverrides` -- Map of chain label to RPC URL for `normalizeTokensWithOverrides`.
- `ChainMetadata` -- Chain metadata (name, short name, explorer URL, etc.).
- `LayerZeroScanConfig` -- Configuration for LZ Scan API (baseUrl, apiKey).
- `FetchWalletStatusParams` -- Parameters for `fetchWalletStatus`.
- `LayerZeroMessageSummary` -- Decoded summary of a LayerZero message.
- `WalletStatusResult` -- Result of `fetchWalletStatus` including messages and pagination.
- `AdaptorBalances` -- Balances stuck in an adaptor contract (underlying, zERC20, native).
- `AdaptorWithdrawParams` -- Parameters for `withdrawFromAdaptor` (writeProvider, adaptorAddress, token, amount).
- `FeeOverrides` -- Gas fee overrides (gasPrice, maxFeePerGas, maxPriorityFeePerGas).

## Constants

| Name | Value | Description |
|------|-------|-------------|
| `AGGREGATION_TREE_HEIGHT` | `6` | Aggregation tree levels |
| `TRANSFER_TREE_HEIGHT` | `40` | Per-chain transfer tree levels |
| `GLOBAL_TRANSFER_TREE_HEIGHT` | `46` | Global tree (40 + 6) |
| `NUM_BATCH_INVOICES` | `10` | Burn addresses per batch invoice |
