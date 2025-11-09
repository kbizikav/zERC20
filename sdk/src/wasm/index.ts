import initWasm, {
  build_full_burn_address as wasmBuildFullBurnAddress,
  decode_full_burn_address as wasmDecodeFullBurnAddress,
  derive_invoice_batch as wasmDeriveInvoiceBatch,
  derive_invoice_single as wasmDeriveInvoiceSingle,
  derive_payment_advice as wasmDerivePaymentAdvice,
  general_recipient_fr as wasmGeneralRecipientFr,
  aggregation_merkle_proof as wasmAggregationMerkleProof,
  aggregation_root as wasmAggregationRoot,
  fetch_aggregation_tree_state as wasmFetchAggregationTreeState,
  fetch_transfer_events as wasmFetchTransferEvents,
  separate_events_by_eligibility as wasmSeparateEventsByEligibility,
  fetch_local_teleport_merkle_proofs as wasmFetchLocalTeleportMerkleProofs,
  generate_global_teleport_merkle_proofs as wasmGenerateGlobalTeleportMerkleProofs,
  SingleWithdrawWasm,
  WithdrawNovaWasm,
  seed_message as wasmSeedMessage,
} from '../assets/wasm/zkerc20_wasm.js';

import {
  AggregationTreeState,
  BurnArtifacts,
  ChainEvents,
  ChainLocalTeleportProofs,
  EventsWithEligibility,
  FetchAggregationTreeStateParams,
  FetchLocalTeleportProofsParams,
  FetchTransferEventsParams,
  GenerateGlobalTeleportProofsParams,
  GlobalTeleportProofWithEvent,
  HubEntryConfig,
  IndexedEvent,
  LocalTeleportProof,
  SecretAndTweak,
  SeparateEventsByEligibilityParams,
  SeparatedChainEvents,
  TokenEntryConfig,
} from '../types.js';
import { normalizeHex, toBigInt } from '../utils/hex.js';

const wasmModuleUrl = new URL('../assets/wasm/zkerc20_wasm_bg.wasm', import.meta.url).toString();

let wasmInitPromise: Promise<void> | undefined;

declare global {
  interface Window {
    __ZKERC20_WASM_PATH__?: string;
  }
}

interface WasmLocatorGlobal {
  __ZKERC20_WASM_PATH__?: string;
  location?: {
    origin?: string;
  };
}

export interface ConfigureWasmOptions {
  baseUrl?: string;
  wasmFilename?: string;
  globalObject?: WasmLocatorGlobal;
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith('/') ? value : `${value}/`;
}

function resolveOriginBase(options: ConfigureWasmOptions): string {
  const baseUrl = ensureTrailingSlash(options.baseUrl ?? '/');
  const hasProtocol = /^https?:\/\//i.test(baseUrl);
  if (hasProtocol) {
    return baseUrl;
  }

  const globalObject = options.globalObject ?? (globalThis as WasmLocatorGlobal | undefined);
  const origin = globalObject?.location?.origin;
  if (!origin) {
    throw new Error('configureWasmLocator requires a global location.origin when baseUrl is relative');
  }

  if (baseUrl.startsWith('/')) {
    return `${origin}${baseUrl}`;
  }
  return `${origin}/${baseUrl}`;
}

export function configureWasmLocator(options: ConfigureWasmOptions = {}): void {
  if (typeof globalThis === 'undefined') {
    return;
  }

  const globalObject = options.globalObject ?? (globalThis as WasmLocatorGlobal);
  if (!options.baseUrl && !options.wasmFilename) {
    globalObject.__ZKERC20_WASM_PATH__ = wasmModuleUrl;
    return;
  }
  const wasmFilename = options.wasmFilename ?? 'assets/wasm/zkerc20_wasm_bg.wasm';
  const base = resolveOriginBase({ ...options, globalObject });
  const wasmUrl = new URL(wasmFilename, base);
  globalObject.__ZKERC20_WASM_PATH__ = wasmUrl.toString();
}

function resolveWasmOverride(): string | undefined {
  try {
    const override = (globalThis as { __ZKERC20_WASM_PATH__?: unknown }).__ZKERC20_WASM_PATH__;
    if (typeof override === 'string') {
      const trimmed = override.trim();
      if (trimmed.length > 0) {
        return trimmed;
      }
    }
  } catch {
    // ignore lookup errors (e.g. globalThis not defined)
  }
  return undefined;
}

function logWasmFallbackWarning(override: string, error: unknown): void {
  if (typeof console === 'undefined' || typeof console.warn !== 'function') {
    return;
  }
  console.warn(
    `zkERC20 wasm failed to load from override path '${override}'. Falling back to the bundled module URL.`,
    error,
  );
}

async function initializeWasm(): Promise<void> {
  const override = resolveWasmOverride();
  if (override) {
    try {
      await initWasm(override);
      return;
    } catch (error) {
      logWasmFallbackWarning(override, error);
    }
  }
  await initWasm(wasmModuleUrl);
}

async function ensureWasm(): Promise<void> {
  if (!wasmInitPromise) {
    wasmInitPromise = initializeWasm();
  }
  await wasmInitPromise;
}

export async function getSeedMessage(): Promise<string> {
  await ensureWasm();
  return wasmSeedMessage();
}

interface RawSecretAndTweak {
  secret: string;
  tweak: string;
}

function asSecretAndTweak(value: unknown): SecretAndTweak {
  const candidate = value as RawSecretAndTweak;
  if (!candidate || typeof candidate.secret !== 'string' || typeof candidate.tweak !== 'string') {
    throw new Error('unexpected secret/tweak payload from wasm');
  }
  return {
    secret: normalizeHex(candidate.secret),
    tweak: normalizeHex(candidate.tweak),
  };
}

interface RawGeneralRecipient {
  chainId: number | string | bigint;
  address: string;
  tweak: string;
  fr: string;
  u256: string;
}

interface RawBurnArtifacts {
  burnAddress: string;
  fullBurnAddress: string;
  secret: string;
  tweak?: string;
  generalRecipient: RawGeneralRecipient;
}

function asBurnArtifacts(value: unknown): BurnArtifacts {
  const raw = value as RawBurnArtifacts;
  if (!raw || typeof raw.burnAddress !== 'string' || typeof raw.secret !== 'string') {
    throw new Error('unexpected burn artifacts payload from wasm');
  }
  const recipient = raw.generalRecipient;
  if (!recipient || typeof recipient.address !== 'string') {
    throw new Error('missing general recipient payload from wasm');
  }
  const tweakSource = raw.tweak ?? recipient.tweak;
  if (typeof tweakSource !== 'string') {
    throw new Error('missing burn tweak payload from wasm');
  }
  return {
    burnAddress: normalizeHex(raw.burnAddress),
    fullBurnAddress: normalizeHex(raw.fullBurnAddress),
    secret: normalizeHex(raw.secret),
    tweak: normalizeHex(tweakSource),
    generalRecipient: {
      chainId: toBigInt(recipient.chainId ?? 0),
      address: normalizeHex(recipient.address),
      tweak: normalizeHex(recipient.tweak),
      fr: normalizeHex(recipient.fr),
      u256: normalizeHex(recipient.u256),
    },
  };
}

type NumericValue = number | string | bigint;

interface RawHubEntry {
  hub_address: string;
  chain_id: NumericValue;
  rpc_urls: string[];
}

interface RawTokenEntry {
  label: string;
  token_address: string;
  verifier_address: string;
  minter_address?: string;
  chain_id: NumericValue;
  deployed_block_number: NumericValue;
  rpc_urls: string[];
  legacy_tx: boolean;
}

interface RawAggregationTreeState {
  latestAggSeq: NumericValue;
  aggregationRoot: string;
  snapshot: string[];
  transferTreeIndices: NumericValue[];
  chainIds: NumericValue[];
}

interface RawIndexedEvent {
  event_index: NumericValue;
  from: string;
  to: string;
  value: string;
  eth_block_number: NumericValue;
}

interface RawChainEvents {
  chainId: NumericValue;
  events: RawIndexedEvent[];
}

interface RawEventsWithEligibility {
  eligible: RawIndexedEvent[];
  ineligible: RawIndexedEvent[];
}

interface RawSeparatedChainEvents {
  chainId: NumericValue;
  events: RawEventsWithEligibility;
}

interface RawLocalTeleportProof {
  treeIndex: NumericValue;
  event: RawIndexedEvent;
  siblings: string[];
}

interface RawChainLocalTeleportProofs {
  chainId: NumericValue;
  proofs: RawLocalTeleportProof[];
}

interface RawGlobalTeleportProof {
  event: RawIndexedEvent;
  siblings: string[];
  leafIndex: NumericValue;
}

function copyRpcUrls(urls: readonly string[], label: string): string[] {
  if (!Array.isArray(urls) || urls.length === 0) {
    throw new Error(`${label} must provide at least one RPC URL`);
  }
  return urls.map((url, idx) => {
    if (typeof url !== 'string' || url.trim().length === 0) {
      throw new Error(`${label} RPC URL at index ${idx} must be a non-empty string`);
    }
    return url.trim();
  });
}

function serializeHubEntry(entry: HubEntryConfig): RawHubEntry {
  return {
    hub_address: normalizeHex(entry.hubAddress),
    chain_id: entry.chainId,
    rpc_urls: copyRpcUrls(entry.rpcUrls, 'hub.rpcUrls'),
  };
}

function serializeTokenEntry(entry: TokenEntryConfig): RawTokenEntry {
  return {
    label: entry.label,
    token_address: normalizeHex(entry.tokenAddress),
    verifier_address: normalizeHex(entry.verifierAddress),
    minter_address: entry.minterAddress ? normalizeHex(entry.minterAddress) : undefined,
    chain_id: entry.chainId,
    deployed_block_number: entry.deployedBlockNumber,
    rpc_urls: copyRpcUrls(entry.rpcUrls, `${entry.label}.rpcUrls`),
    legacy_tx: entry.legacyTx ?? false,
  };
}

function serializeAggregationTreeState(state: AggregationTreeState): RawAggregationTreeState {
  return {
    latestAggSeq: state.latestAggSeq,
    aggregationRoot: normalizeHex(state.aggregationRoot),
    snapshot: state.snapshot.map((value) => normalizeHex(value)),
    transferTreeIndices: state.transferTreeIndices.map((value) => value),
    chainIds: state.chainIds.map((value) => value),
  };
}

function deserializeAggregationTreeState(raw: RawAggregationTreeState): AggregationTreeState {
  return {
    latestAggSeq: toBigInt(raw.latestAggSeq),
    aggregationRoot: normalizeHex(raw.aggregationRoot),
    snapshot: raw.snapshot.map((value) => normalizeHex(value)),
    transferTreeIndices: raw.transferTreeIndices.map((value) => toBigInt(value)),
    chainIds: raw.chainIds.map((value) => toBigInt(value)),
  };
}

function serializeIndexedEvent(event: IndexedEvent): RawIndexedEvent {
  return {
    event_index: event.eventIndex,
    from: normalizeHex(event.from),
    to: normalizeHex(event.to),
    value: normalizeHex(event.value),
    eth_block_number: event.ethBlockNumber,
  };
}

function deserializeIndexedEvent(raw: RawIndexedEvent): IndexedEvent {
  return {
    eventIndex: toBigInt(raw.event_index),
    from: normalizeHex(raw.from),
    to: normalizeHex(raw.to),
    value: toBigInt(raw.value),
    ethBlockNumber: toBigInt(raw.eth_block_number),
  };
}

function serializeChainEvents(entry: ChainEvents): RawChainEvents {
  return {
    chainId: entry.chainId,
    events: entry.events.map(serializeIndexedEvent),
  };
}

function deserializeChainEvents(raw: RawChainEvents): ChainEvents {
  return {
    chainId: toBigInt(raw.chainId),
    events: raw.events.map(deserializeIndexedEvent),
  };
}

function deserializeEventsWithEligibility(raw: RawEventsWithEligibility): EventsWithEligibility {
  return {
    eligible: raw.eligible.map(deserializeIndexedEvent),
    ineligible: raw.ineligible.map(deserializeIndexedEvent),
  };
}

function deserializeSeparatedChainEvents(raw: RawSeparatedChainEvents): SeparatedChainEvents {
  return {
    chainId: toBigInt(raw.chainId),
    events: deserializeEventsWithEligibility(raw.events),
  };
}

function serializeLocalTeleportProof(proof: LocalTeleportProof): RawLocalTeleportProof {
  return {
    treeIndex: proof.treeIndex,
    event: serializeIndexedEvent(proof.event),
    siblings: proof.siblings.map((value) => normalizeHex(value)),
  };
}

function deserializeLocalTeleportProof(raw: RawLocalTeleportProof): LocalTeleportProof {
  return {
    treeIndex: toBigInt(raw.treeIndex),
    event: deserializeIndexedEvent(raw.event),
    siblings: raw.siblings.map((value) => normalizeHex(value)),
  };
}

function serializeChainLocalTeleportProofs(
  entry: ChainLocalTeleportProofs,
): RawChainLocalTeleportProofs {
  return {
    chainId: entry.chainId,
    proofs: entry.proofs.map(serializeLocalTeleportProof),
  };
}

function deserializeGlobalTeleportProof(raw: RawGlobalTeleportProof): GlobalTeleportProofWithEvent {
  return {
    event: deserializeIndexedEvent(raw.event),
    siblings: raw.siblings.map((value) => normalizeHex(value)),
    leafIndex: toBigInt(raw.leafIndex),
  };
}

function toSafeNumber(value: number | bigint, label: string): number {
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || !Number.isInteger(value)) {
      throw new Error(`${label} must be a finite integer`);
    }
    return value;
  }
  if (value < 0n) {
    throw new Error(`${label} must be non-negative`);
  }
  const asNumber = Number(value);
  if (!Number.isFinite(asNumber) || BigInt(asNumber) !== value) {
    throw new Error(`${label} exceeds JavaScript safe integer range`);
  }
  return asNumber;
}

export async function derivePaymentAdvice(
  seedHex: string,
  paymentAdviceIdHex: string,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  await ensureWasm();
  return asSecretAndTweak(
    wasmDerivePaymentAdvice(
      normalizeHex(seedHex),
      normalizeHex(paymentAdviceIdHex),
      toBigInt(recipientChainId),
      normalizeHex(recipientAddress),
    ),
  );
}

export async function deriveInvoiceSingle(
  seedHex: string,
  invoiceIdHex: string,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  await ensureWasm();
  return asSecretAndTweak(
    wasmDeriveInvoiceSingle(
      normalizeHex(seedHex),
      normalizeHex(invoiceIdHex),
      toBigInt(recipientChainId),
      normalizeHex(recipientAddress),
    ),
  );
}

export async function deriveInvoiceBatch(
  seedHex: string,
  invoiceIdHex: string,
  subId: number,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  await ensureWasm();
  return asSecretAndTweak(
    wasmDeriveInvoiceBatch(
      normalizeHex(seedHex),
      normalizeHex(invoiceIdHex),
      subId,
      toBigInt(recipientChainId),
      normalizeHex(recipientAddress),
    ),
  );
}

export async function buildFullBurnAddress(
  recipientChainId: bigint | number,
  recipientAddress: string,
  secretHex: string,
  tweakHex: string,
): Promise<BurnArtifacts> {
  await ensureWasm();
  const result = wasmBuildFullBurnAddress(
    toBigInt(recipientChainId),
    normalizeHex(recipientAddress),
    normalizeHex(secretHex),
    normalizeHex(tweakHex),
  );
  return asBurnArtifacts(result);
}

export async function decodeFullBurnAddress(payloadHex: string): Promise<BurnArtifacts> {
  await ensureWasm();
  return asBurnArtifacts(wasmDecodeFullBurnAddress(normalizeHex(payloadHex)));
}

export async function generalRecipientFr(
  chainId: bigint | number,
  recipientAddress: string,
  tweakHex: string,
): Promise<string> {
  await ensureWasm();
  return normalizeHex(
    wasmGeneralRecipientFr(toBigInt(chainId), normalizeHex(recipientAddress), normalizeHex(tweakHex)),
  );
}

export async function aggregationRoot(snapshot: readonly string[]): Promise<string> {
  await ensureWasm();
  return normalizeHex(wasmAggregationRoot(snapshot.slice()));
}

export async function aggregationMerkleProof(snapshot: readonly string[], index: number): Promise<string[]> {
  await ensureWasm();
  const siblings: string[] = wasmAggregationMerkleProof(snapshot.slice(), index);
  return siblings.map((value) => normalizeHex(value));
}

export async function fetchAggregationTreeState(
  params: FetchAggregationTreeStateParams,
): Promise<AggregationTreeState> {
  await ensureWasm();
  const payload: {
    eventBlockSpan?: number;
    hub: RawHubEntry;
    token: RawTokenEntry;
  } = {
    hub: serializeHubEntry(params.hub),
    token: serializeTokenEntry(params.token),
  };
  if (params.eventBlockSpan !== undefined) {
    payload.eventBlockSpan = toSafeNumber(params.eventBlockSpan, 'eventBlockSpan');
  }
  const rawState: RawAggregationTreeState = await wasmFetchAggregationTreeState(payload);
  return deserializeAggregationTreeState(rawState);
}

export async function fetchTransferEvents(params: FetchTransferEventsParams): Promise<ChainEvents[]> {
  await ensureWasm();
  const payload = {
    indexerUrl: params.indexerUrl,
    indexerFetchLimit: params.indexerFetchLimit,
    tokens: params.tokens.map(serializeTokenEntry),
    burnAddresses: params.burnAddresses.map((address) => normalizeHex(address)),
  };
  const rawEvents: RawChainEvents[] = await wasmFetchTransferEvents(payload);
  return rawEvents.map((entry) => deserializeChainEvents(entry));
}

export async function separateEventsByEligibility(
  params: SeparateEventsByEligibilityParams,
): Promise<SeparatedChainEvents[]> {
  await ensureWasm();
  const payload = {
    aggregationState: serializeAggregationTreeState(params.aggregationState),
    events: params.events.map(serializeChainEvents),
  };
  const rawSeparated: RawSeparatedChainEvents[] = wasmSeparateEventsByEligibility(payload);
  return rawSeparated.map((entry) => deserializeSeparatedChainEvents(entry));
}

export async function fetchLocalTeleportMerkleProofs(
  params: FetchLocalTeleportProofsParams,
): Promise<LocalTeleportProof[]> {
  await ensureWasm();
  const payload = {
    indexerUrl: params.indexerUrl,
    token: serializeTokenEntry(params.token),
    treeIndex: toBigInt(params.treeIndex),
    events: params.events.map(serializeIndexedEvent),
  };
  const rawProofs: RawLocalTeleportProof[] = await wasmFetchLocalTeleportMerkleProofs(payload);
  return rawProofs.map((proof) => deserializeLocalTeleportProof(proof));
}

export async function generateGlobalTeleportMerkleProofs(
  params: GenerateGlobalTeleportProofsParams,
): Promise<GlobalTeleportProofWithEvent[]> {
  await ensureWasm();
  const payload = {
    aggregationState: serializeAggregationTreeState(params.aggregationState),
    proofs: params.chains.map(serializeChainLocalTeleportProofs),
  };
  const rawProofs: RawGlobalTeleportProof[] = wasmGenerateGlobalTeleportMerkleProofs(payload);
  return rawProofs.map((proof) => deserializeGlobalTeleportProof(proof));
}

export async function createSingleWithdrawWasm(
  localPk: Uint8Array,
  localVk: Uint8Array,
  globalPk: Uint8Array,
  globalVk: Uint8Array,
): Promise<SingleWithdrawWasm> {
  await ensureWasm();
  return new SingleWithdrawWasm(localPk, localVk, globalPk, globalVk);
}

export async function createWithdrawNovaWasm(
  localPp: Uint8Array,
  localVp: Uint8Array,
  globalPp: Uint8Array,
  globalVp: Uint8Array,
): Promise<WithdrawNovaWasm> {
  await ensureWasm();
  return new WithdrawNovaWasm(localPp, localVp, globalPp, globalVp);
}

export type { SingleWithdrawWasm, WithdrawNovaWasm };
