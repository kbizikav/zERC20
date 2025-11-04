import initWasm, {
  build_full_burn_address as wasmBuildFullBurnAddress,
  decode_full_burn_address as wasmDecodeFullBurnAddress,
  derive_invoice_batch as wasmDeriveInvoiceBatch,
  derive_invoice_single as wasmDeriveInvoiceSingle,
  derive_payment_advice as wasmDerivePaymentAdvice,
  general_recipient_fr as wasmGeneralRecipientFr,
  aggregation_merkle_proof as wasmAggregationMerkleProof,
  aggregation_root as wasmAggregationRoot,
  SingleWithdrawWasm,
  WithdrawNovaWasm,
  seed_message as wasmSeedMessage,
} from '@/assets/wasm/zkerc20_wasm.js';
import wasmModuleUrl from '@/assets/wasm/zkerc20_wasm_bg.wasm?url';

import { BurnArtifacts, SecretAndTweak } from '../core/types.js';
import { normalizeHex, toBigInt } from '../core/utils.js';

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
  const wasmFilename = options.wasmFilename ?? 'wasm/zkerc20_wasm_bg.wasm';
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
