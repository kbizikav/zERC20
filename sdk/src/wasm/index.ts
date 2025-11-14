import type { SingleWithdrawWasm, WithdrawNovaWasm } from '../assets/wasm/web/zkerc20_wasm.js';

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
import { NUM_BATCH_INVOICES } from '../constants.js';

const DEFAULT_WASM_FILENAME = 'assets/wasm/web/zkerc20_wasm_bg.wasm';
const BUNDLED_WASM_URL = new URL('../assets/wasm/web/zkerc20_wasm_bg.wasm', import.meta.url).toString();

type WebWasmBindings = typeof import('../assets/wasm/web/zkerc20_wasm.js');
type CommonWasmBindings = Omit<WebWasmBindings, 'default' | 'initSync'> &
  Partial<Pick<WebWasmBindings, 'default' | 'initSync'>>;
type WasmBindings = CommonWasmBindings;
type WebInitFn = NonNullable<WebWasmBindings['default']>;
type NodeModule = typeof import('node:module');

let bindingsCache: WasmBindings | undefined;
let bindingsPromise: Promise<WasmBindings> | undefined;

function isNodeRuntime(): boolean {
  return typeof process !== 'undefined' && typeof process.versions?.node === 'string';
}

async function getOrLoadBindings(): Promise<WasmBindings> {
  if (bindingsCache) {
    return bindingsCache;
  }
  if (!bindingsPromise) {
    bindingsPromise = resolveBindings();
  }
  bindingsCache = await bindingsPromise;
  return bindingsCache;
}

async function resolveBindings(): Promise<WasmBindings> {
  if (isNodeRuntime()) {
    return loadNodeBindings();
  }
  return import('../assets/wasm/web/zkerc20_wasm.js');
}

async function loadNodeBindings(): Promise<WasmBindings> {
  const { createRequire } = await loadNodeModule();
  const require = createRequire(import.meta.url);
  return require('../assets/wasm/node/zkerc20_wasm.js') as WasmBindings;
}

async function loadNodeModule(): Promise<NodeModule> {
  const specifiers = ['node:module', 'module'] as const;
  let lastError: unknown;
  for (const specifier of specifiers) {
    try {
      const mod = (await import(specifier as string)) as NodeModule;
      return mod;
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError ?? new Error('failed to load node:module for wasm bindings');
}

function requiresExplicitInit(bindings: WasmBindings): bindings is WasmBindings & { default: WebInitFn } {
  return typeof bindings.default === 'function';
}

export interface WasmLocatorGlobal {
  location?: {
    origin?: string;
  };
}

export interface WasmRuntimeOptions {
  /** Direct URL to the wasm bundle. */
  url?: string;
  /** Base URL used to resolve the wasm file; relative paths rely on window.location.origin. */
  baseUrl?: string;
  /** Override filename when using a custom base. */
  wasmFilename?: string;
  /** Custom global object used when resolving relative base URLs (defaults to globalThis). */
  globalObject?: WasmLocatorGlobal;
}

export class WasmRuntime {
  private locator: WasmRuntimeOptions;
  private overrideUrl?: string;
  private initPromise?: Promise<void>;
  private wasmBindings?: WasmBindings;

  constructor(options: WasmRuntimeOptions = {}) {
    this.locator = { ...options };
    this.overrideUrl = normalizeOverride(options.url);
  }

  configure(options: WasmRuntimeOptions): void {
    this.locator = { ...options };
    this.overrideUrl = normalizeOverride(options.url);
    this.initPromise = undefined;
  }

  setOverrideUrl(url?: string): void {
    this.overrideUrl = normalizeOverride(url);
    this.initPromise = undefined;
  }

  async ready(): Promise<void> {
    await this.ensureReady();
  }

  async getSeedMessage(): Promise<string> {
    const wasm = await this.readyBindings();
    return wasm.seed_message();
  }

  async derivePaymentAdvice(
    seedHex: string,
    paymentAdviceIdHex: string,
    recipientChainId: bigint | number,
    recipientAddress: string,
  ): Promise<SecretAndTweak> {
    const wasm = await this.readyBindings();
    return asSecretAndTweak(
      wasm.derive_payment_advice(
        normalizeHex(seedHex),
        normalizeHex(paymentAdviceIdHex),
        toBigInt(recipientChainId),
        normalizeHex(recipientAddress),
      ),
    );
  }

  async deriveInvoiceSingle(
    seedHex: string,
    invoiceIdHex: string,
    recipientChainId: bigint | number,
    recipientAddress: string,
  ): Promise<SecretAndTweak> {
    const wasm = await this.readyBindings();
    return asSecretAndTweak(
      wasm.derive_invoice_single(
        normalizeHex(seedHex),
        normalizeHex(invoiceIdHex),
        toBigInt(recipientChainId),
        normalizeHex(recipientAddress),
      ),
    );
  }

  async deriveInvoiceBatch(
    seedHex: string,
    invoiceIdHex: string,
    subId: number,
    recipientChainId: bigint | number,
    recipientAddress: string,
  ): Promise<SecretAndTweak> {
    assertValidInvoiceSubId(subId);
    const wasm = await this.readyBindings();
    return asSecretAndTweak(
      wasm.derive_invoice_batch(
        normalizeHex(seedHex),
        normalizeHex(invoiceIdHex),
        subId,
        toBigInt(recipientChainId),
        normalizeHex(recipientAddress),
      ),
    );
  }

  async buildFullBurnAddress(
    recipientChainId: bigint | number,
    recipientAddress: string,
    secretHex: string,
    tweakHex: string,
  ): Promise<BurnArtifacts> {
    const wasm = await this.readyBindings();
    const result = wasm.build_full_burn_address(
      toBigInt(recipientChainId),
      normalizeHex(recipientAddress),
      normalizeHex(secretHex),
      normalizeHex(tweakHex),
    );
    return asBurnArtifacts(result);
  }

  async decodeFullBurnAddress(payloadHex: string): Promise<BurnArtifacts> {
    const wasm = await this.readyBindings();
    return asBurnArtifacts(wasm.decode_full_burn_address(normalizeHex(payloadHex)));
  }

  async generalRecipientFr(
    chainId: bigint | number,
    recipientAddress: string,
    tweakHex: string,
  ): Promise<string> {
    const wasm = await this.readyBindings();
    return normalizeHex(
      wasm.general_recipient_fr(toBigInt(chainId), normalizeHex(recipientAddress), normalizeHex(tweakHex)),
    );
  }

  async aggregationRoot(snapshot: readonly string[]): Promise<string> {
    const wasm = await this.readyBindings();
    return normalizeHex(wasm.aggregation_root(snapshot.slice()));
  }

  async aggregationMerkleProof(snapshot: readonly string[], index: number): Promise<string[]> {
    const wasm = await this.readyBindings();
    const siblings: string[] = wasm.aggregation_merkle_proof(snapshot.slice(), index);
    return siblings.map((value) => normalizeHex(value));
  }

  async fetchAggregationTreeState(params: FetchAggregationTreeStateParams): Promise<AggregationTreeState> {
    const wasm = await this.readyBindings();
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
    const rawState: RawAggregationTreeState = await wasm.fetch_aggregation_tree_state(payload);
    return deserializeAggregationTreeState(rawState);
  }

  async fetchTransferEvents(params: FetchTransferEventsParams): Promise<ChainEvents[]> {
    const wasm = await this.readyBindings();
    const payload = {
      indexerUrl: params.indexerUrl,
      indexerFetchLimit: params.indexerFetchLimit,
      tokens: params.tokens.map(serializeTokenEntry),
      burnAddresses: params.burnAddresses.map((address) => normalizeHex(address)),
    };
    const rawEvents: RawChainEvents[] = await wasm.fetch_transfer_events(payload);
    return rawEvents.map((entry) => deserializeChainEvents(entry));
  }

  async separateEventsByEligibility(
    params: SeparateEventsByEligibilityParams,
  ): Promise<SeparatedChainEvents[]> {
    const wasm = await this.readyBindings();
    const payload = {
      aggregationState: serializeAggregationTreeState(params.aggregationState),
      events: params.events.map(serializeChainEvents),
    };
    const rawSeparated: RawSeparatedChainEvents[] = wasm.separate_events_by_eligibility(payload);
    return rawSeparated.map((entry) => deserializeSeparatedChainEvents(entry));
  }

  async fetchLocalTeleportMerkleProofs(
    params: FetchLocalTeleportProofsParams,
  ): Promise<LocalTeleportProof[]> {
    const wasm = await this.readyBindings();
    const payload = {
      indexerUrl: params.indexerUrl,
      token: serializeTokenEntry(params.token),
      treeIndex: toBigInt(params.treeIndex),
      events: params.events.map(serializeIndexedEvent),
    };
    const rawProofs: RawLocalTeleportProof[] = await wasm.fetch_local_teleport_merkle_proofs(payload);
    return rawProofs.map((proof) => deserializeLocalTeleportProof(proof));
  }

  async generateGlobalTeleportMerkleProofs(
    params: GenerateGlobalTeleportProofsParams,
  ): Promise<GlobalTeleportProofWithEvent[]> {
    const wasm = await this.readyBindings();
    const payload = {
      aggregationState: serializeAggregationTreeState(params.aggregationState),
      proofs: params.chains.map(serializeChainLocalTeleportProofs),
    };
    const rawProofs: RawGlobalTeleportProof[] = wasm.generate_global_teleport_merkle_proofs(payload);
    return rawProofs.map((proof) => deserializeGlobalTeleportProof(proof));
  }

  async createSingleWithdrawProgram(
    localPk: Uint8Array,
    localVk: Uint8Array,
    globalPk: Uint8Array,
    globalVk: Uint8Array,
  ): Promise<SingleWithdrawWasm> {
    const wasm = await this.readyBindings();
    return new wasm.SingleWithdrawWasm(localPk, localVk, globalPk, globalVk);
  }

  async createWithdrawNovaProgram(
    localPp: Uint8Array,
    localVp: Uint8Array,
    globalPp: Uint8Array,
    globalVp: Uint8Array,
  ): Promise<WithdrawNovaWasm> {
    const wasm = await this.readyBindings();
    return new wasm.WithdrawNovaWasm(localPp, localVp, globalPp, globalVp);
  }

  private async ensureReady(): Promise<void> {
    if (!this.initPromise) {
      this.initPromise = this.initialize();
    }
    await this.initPromise;
  }

  private async readyBindings(): Promise<WasmBindings> {
    await this.ensureReady();
    return this.getBindings();
  }

  private getBindings(): WasmBindings {
    if (!this.wasmBindings) {
      throw new Error('zkERC20 wasm bindings are not initialized');
    }
    return this.wasmBindings;
  }

  private async initialize(): Promise<void> {
    const bindings = await getOrLoadBindings();
    if (!requiresExplicitInit(bindings)) {
      this.wasmBindings = bindings;
      return;
    }

    const candidates = this.resolveCandidateUrls();
    let lastError: unknown;
    for (let idx = 0; idx < candidates.length; idx++) {
      const candidate = candidates[idx];
      try {
        await bindings.default({ module_or_path: candidate });
        this.wasmBindings = bindings;
        return;
      } catch (error) {
        lastError = error;
        if (idx < candidates.length - 1) {
          logWasmFallbackWarning(candidate, error);
        }
      }
    }
    throw new Error(`failed to initialize zkERC20 wasm: ${formatError(lastError)}`);
  }

  private resolveCandidateUrls(): string[] {
    const urls: string[] = [];
    if (this.overrideUrl) {
      urls.push(this.overrideUrl);
    }
    const configured = this.resolveConfiguredUrl();
    if (configured && configured !== this.overrideUrl) {
      urls.push(configured);
    }
    if (!urls.includes(BUNDLED_WASM_URL)) {
      urls.push(BUNDLED_WASM_URL);
    }
    return urls;
  }

  private resolveConfiguredUrl(): string | undefined {
    const baseUrl = this.locator.baseUrl;
    if (!baseUrl) {
      return undefined;
    }
    const wasmFilename = this.locator.wasmFilename ?? DEFAULT_WASM_FILENAME;
    if (/^https?:\/\//i.test(baseUrl)) {
      return new URL(wasmFilename, ensureTrailingSlash(baseUrl)).toString();
    }
    const globalRef = this.locator.globalObject ?? (typeof globalThis !== 'undefined' ? (globalThis as WasmLocatorGlobal) : undefined);
    const origin = globalRef?.location?.origin;
    if (!origin) {
      throw new Error('WasmRuntime requires a global location.origin when baseUrl is relative');
    }
    const prefix = baseUrl.startsWith('/') ? `${origin}${baseUrl}` : `${origin}/${baseUrl}`;
    return new URL(wasmFilename, ensureTrailingSlash(prefix)).toString();
  }
}

const defaultRuntime = new WasmRuntime();

function runtime(): WasmRuntime {
  return defaultRuntime;
}

export function getDefaultWasmRuntime(): WasmRuntime {
  return runtime();
}

export function configureWasmLocator(options: WasmRuntimeOptions = {}): void {
  runtime().configure(options);
}

export async function getSeedMessage(): Promise<string> {
  return runtime().getSeedMessage();
}

export async function derivePaymentAdvice(
  seedHex: string,
  paymentAdviceIdHex: string,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  return runtime().derivePaymentAdvice(seedHex, paymentAdviceIdHex, recipientChainId, recipientAddress);
}

export async function deriveInvoiceSingle(
  seedHex: string,
  invoiceIdHex: string,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  return runtime().deriveInvoiceSingle(seedHex, invoiceIdHex, recipientChainId, recipientAddress);
}

export async function deriveInvoiceBatch(
  seedHex: string,
  invoiceIdHex: string,
  subId: number,
  recipientChainId: bigint | number,
  recipientAddress: string,
): Promise<SecretAndTweak> {
  return runtime().deriveInvoiceBatch(seedHex, invoiceIdHex, subId, recipientChainId, recipientAddress);
}

export async function buildFullBurnAddress(
  recipientChainId: bigint | number,
  recipientAddress: string,
  secretHex: string,
  tweakHex: string,
): Promise<BurnArtifacts> {
  return runtime().buildFullBurnAddress(recipientChainId, recipientAddress, secretHex, tweakHex);
}

export async function decodeFullBurnAddress(payloadHex: string): Promise<BurnArtifacts> {
  return runtime().decodeFullBurnAddress(payloadHex);
}

export async function generalRecipientFr(
  chainId: bigint | number,
  recipientAddress: string,
  tweakHex: string,
): Promise<string> {
  return runtime().generalRecipientFr(chainId, recipientAddress, tweakHex);
}

export async function aggregationRoot(snapshot: readonly string[]): Promise<string> {
  return runtime().aggregationRoot(snapshot);
}

export async function aggregationMerkleProof(snapshot: readonly string[], index: number): Promise<string[]> {
  return runtime().aggregationMerkleProof(snapshot, index);
}

export async function fetchAggregationTreeState(
  params: FetchAggregationTreeStateParams,
): Promise<AggregationTreeState> {
  return runtime().fetchAggregationTreeState(params);
}

export async function fetchTransferEvents(params: FetchTransferEventsParams): Promise<ChainEvents[]> {
  return runtime().fetchTransferEvents(params);
}

export async function separateEventsByEligibility(
  params: SeparateEventsByEligibilityParams,
): Promise<SeparatedChainEvents[]> {
  return runtime().separateEventsByEligibility(params);
}

export async function fetchLocalTeleportMerkleProofs(
  params: FetchLocalTeleportProofsParams,
): Promise<LocalTeleportProof[]> {
  return runtime().fetchLocalTeleportMerkleProofs(params);
}

export async function generateGlobalTeleportMerkleProofs(
  params: GenerateGlobalTeleportProofsParams,
): Promise<GlobalTeleportProofWithEvent[]> {
  return runtime().generateGlobalTeleportMerkleProofs(params);
}

export async function createSingleWithdrawWasm(
  localPk: Uint8Array,
  localVk: Uint8Array,
  globalPk: Uint8Array,
  globalVk: Uint8Array,
): Promise<SingleWithdrawWasm> {
  return runtime().createSingleWithdrawProgram(localPk, localVk, globalPk, globalVk);
}

export async function createWithdrawNovaWasm(
  localPp: Uint8Array,
  localVp: Uint8Array,
  globalPp: Uint8Array,
  globalVp: Uint8Array,
): Promise<WithdrawNovaWasm> {
  return runtime().createWithdrawNovaProgram(localPp, localVp, globalPp, globalVp);
}

function normalizeOverride(url?: string): string | undefined {
  if (!url) {
    return undefined;
  }
  const trimmed = url.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function ensureTrailingSlash(value: string): string {
  return value.endsWith('/') ? value : `${value}/`;
}

function logWasmFallbackWarning(url: string, error: unknown): void {
  if (typeof console === 'undefined' || typeof console.warn !== 'function') {
    return;
  }
  console.warn(
    `zkERC20 wasm failed to load from '${url}'. Falling back to the next candidate.`,
    error,
  );
}

function formatError(error: unknown): string {
  if (!error) {
    return 'unknown error';
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
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

function assertValidInvoiceSubId(subId: number): void {
  if (!Number.isFinite(subId) || !Number.isInteger(subId)) {
    throw new Error('subId must be a finite integer');
  }
  if (subId < 0 || subId >= NUM_BATCH_INVOICES) {
    throw new Error(`subId must be between 0 and ${NUM_BATCH_INVOICES - 1}`);
  }
}

function toSafeNumber(value: number | bigint, label: string): number {
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || !Number.isInteger(value)) {
      throw new Error(`${label} must be a finite integer`);
    }
    if (value < 0) {
      throw new Error(`${label} must be non-negative`);
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

export type { SingleWithdrawWasm, WithdrawNovaWasm };
