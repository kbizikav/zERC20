import { JsonRpcProvider } from 'ethers';
import { decode as decodeBase64 } from 'base64-arraybuffer';
import { ungzip } from 'pako';

import { normalizeHex, toBigInt } from '../utils/hex.js';

export interface TokenEntry {
  label: string;
  tokenAddress: string;
  verifierAddress: string;
  minterAddress?: string;
  chainId: bigint;
  deployedBlockNumber: bigint;
  rpcUrls: string[];
  legacyTx: boolean;
}

export interface HubEntry {
  hubAddress: string;
  chainId: bigint;
  rpcUrls: string[];
}

export interface TokensFile {
  hub?: HubEntry;
  tokens: TokenEntry[];
}

export interface NormalizedTokens {
  hub?: HubEntry;
  tokens: TokenEntry[];
  raw: TokensFile;
}

type MaybeString = string | number | bigint;

function toHexAddress(value: string): string {
  return normalizeHex(value);
}

function parseBigInt(value: MaybeString, label: string): bigint {
  try {
    return toBigInt(value);
  } catch {
    throw new Error(`${label} must be a bigint-compatible value`);
  }
}

function normalizeRpcUrls(urls: unknown, label: string): string[] {
  if (!Array.isArray(urls) || urls.length === 0) {
    throw new Error(`${label} must configure at least one rpc url`);
  }
  return urls.map((url) => {
    if (typeof url !== 'string' || url.trim().length === 0) {
      throw new Error(`${label} contains invalid rpc url`);
    }
    return url.trim();
  });
}

type UnknownRecord = Record<string, unknown>;

function expectRecord(value: unknown, label: string): UnknownRecord {
  if (!value || typeof value !== 'object') {
    throw new Error(`${label} must be an object`);
  }
  return value as UnknownRecord;
}

function getStringField(source: UnknownRecord, keys: string[], label: string): string {
  for (const key of keys) {
    const candidate = source[key];
    if (typeof candidate === 'string') {
      const trimmed = candidate.trim();
      if (trimmed.length === 0) {
        throw new Error(`${label} must be a non-empty string`);
      }
      return trimmed;
    }
  }
  throw new Error(`${label} must be a string`);
}

function getOptionalStringField(source: UnknownRecord, keys: string[], label: string): string | undefined {
  for (const key of keys) {
    if (key in source) {
      const candidate = source[key];
      if (candidate === undefined || candidate === null) {
        return undefined;
      }
      if (typeof candidate !== 'string') {
        throw new Error(`${label} must be a string when provided`);
      }
      const trimmed = candidate.trim();
      if (!trimmed) {
        throw new Error(`${label} must be a non-empty string when provided`);
      }
      return trimmed;
    }
  }
  return undefined;
}

function getValueField<T>(
  source: UnknownRecord,
  keys: string[],
  label: string,
): T {
  for (const key of keys) {
    if (key in source) {
      const candidate = source[key];
      if (candidate !== undefined && candidate !== null) {
        return candidate as T;
      }
    }
  }
  throw new Error(`${label} is required`);
}

function getOptionalBooleanField(
  source: UnknownRecord,
  keys: string[],
  label: string,
): boolean | undefined {
  for (const key of keys) {
    if (key in source) {
      const candidate = source[key];
      if (typeof candidate === 'boolean') {
        return candidate;
      }
      throw new Error(`${label} must be a boolean`);
    }
  }
  return undefined;
}

export function normalizeTokensFile(file: TokensFile): TokensFile {
  if (!Array.isArray(file.tokens) || file.tokens.length === 0) {
    throw new Error('tokens array must be non-empty');
  }
  file.tokens = file.tokens.map((entry) => {
    const record = expectRecord(entry, 'token entry');
    const label = getStringField(record, ['label'], 'token label');
    const minterAddressValue = getOptionalStringField(
      record,
      ['minterAddress', 'minter_address'],
      `${label}.minterAddress`,
    );
    return {
      label,
      tokenAddress: toHexAddress(
        getStringField(record, ['tokenAddress', 'token_address'], `${label}.tokenAddress`),
      ),
      verifierAddress: toHexAddress(
        getStringField(record, ['verifierAddress', 'verifier_address'], `${label}.verifierAddress`),
      ),
      minterAddress: minterAddressValue ? toHexAddress(minterAddressValue) : undefined,
      chainId: parseBigInt(
        getValueField<MaybeString>(record, ['chainId', 'chain_id'], `${label}.chainId`),
        `${label}.chainId`,
      ),
      deployedBlockNumber: parseBigInt(
        getValueField<MaybeString>(
          record,
          ['deployedBlockNumber', 'deployed_block_number'],
          `${label}.deployedBlockNumber`,
        ),
        `${label}.deployedBlockNumber`,
      ),
      rpcUrls: normalizeRpcUrls(
        getValueField<unknown>(record, ['rpcUrls', 'rpc_urls'], `${label}.rpcUrls`),
        label,
      ),
      legacyTx:
        getOptionalBooleanField(record, ['legacyTx', 'legacy_tx'], `${label}.legacyTx`) ?? false,
    };
  });

  if (file.hub) {
    const record = expectRecord(file.hub, 'hub entry');
    file.hub = {
      hubAddress: toHexAddress(getStringField(record, ['hubAddress', 'hub_address'], 'hub.hubAddress')),
      chainId: parseBigInt(getValueField<MaybeString>(record, ['chainId', 'chain_id'], 'hub.chainId'), 'hub.chainId'),
      rpcUrls: normalizeRpcUrls(getValueField<unknown>(record, ['rpcUrls', 'rpc_urls'], 'hub.rpcUrls'), 'hub'),
    };
  }

  return file;
}

function cloneTokensFile(file: TokensFile): TokensFile {
  return {
    ...file,
    tokens: Array.isArray(file.tokens) ? [...file.tokens] : [],
    hub: file.hub ? { ...file.hub } : undefined,
  };
}

function asNormalizedTokens(file: TokensFile): NormalizedTokens {
  const normalized = normalizeTokensFile(cloneTokensFile(file));
  return {
    raw: file,
    tokens: normalized.tokens,
    hub: normalized.hub,
  };
}

function base64ToBytes(value: string): Uint8Array {
  const normalized = value.replace(/\s+/g, '');
  if (!normalized) {
    throw new Error('Compressed tokens payload is empty');
  }
  try {
    const buffer = decodeBase64(normalized);
    return new Uint8Array(buffer);
  } catch {
    throw new Error('Compressed tokens payload is not valid base64');
  }
}

const tokenCache = new Map<string, Promise<NormalizedTokens>>();

export interface LoadTokensOptions {
  cacheKey?: string;
  cache?: Map<string, Promise<NormalizedTokens>>;
  decoder?: TextDecoder;
  decompress?: (data: Uint8Array) => Uint8Array | ArrayBuffer;
}

function decodeTokensPayload(bytes: Uint8Array, decoder?: TextDecoder): TokensFile {
  const textDecoder = decoder ?? new TextDecoder();
  const text = textDecoder.decode(bytes);
  try {
    return JSON.parse(text) as TokensFile;
  } catch {
    throw new Error('Decompressed tokens blob is not valid JSON');
  }
}

export function normalizeTokens(file: TokensFile): NormalizedTokens {
  return asNormalizedTokens(file);
}

export function clearTokensCache(cache?: Map<string, Promise<NormalizedTokens>>, key?: string): void {
  const target = cache ?? tokenCache;
  if (key) {
    target.delete(key);
    return;
  }
  target.clear();
}

export function loadTokensFromCompressed(
  compressed: string,
  options: LoadTokensOptions = {},
): Promise<NormalizedTokens> {
  const cache = options.cache ?? tokenCache;
  const cacheKey = options.cacheKey ?? `compressed:${compressed}`;
  if (cache.has(cacheKey)) {
    return cache.get(cacheKey) as Promise<NormalizedTokens>;
  }

  const promise = Promise.resolve().then(() => {
    const gzippedBytes = base64ToBytes(compressed);
    const decompress = options.decompress ?? ((data: Uint8Array) => ungzip(data));
    const decompressed = decompress(gzippedBytes);
    const normalizedBytes =
      decompressed instanceof Uint8Array ? decompressed : new Uint8Array(decompressed as ArrayBufferLike);
    const parsed = decodeTokensPayload(normalizedBytes, options.decoder);
    return asNormalizedTokens(parsed);
  });

  cache.set(cacheKey, promise);
  return promise;
}

export function loadTokens(
  compressed: string,
  options: LoadTokensOptions = {},
): Promise<NormalizedTokens> {
  return loadTokensFromCompressed(compressed, options);
}

export function findTokenByChain(tokens: readonly TokenEntry[], chainId: bigint): TokenEntry {
  const matches = tokens.filter((token) => token.chainId === chainId);
  if (matches.length === 0) {
    throw new Error(`no tokens configured for chain id ${chainId}`);
  }
  if (matches.length > 1) {
    throw new Error(`multiple tokens configured for chain id ${chainId}; disambiguate by label`);
  }
  return matches[0];
}

export function createProviderForToken(entry: TokenEntry): JsonRpcProvider {
  if (entry.rpcUrls.length === 0) {
    throw new Error(`token '${entry.label}' is missing rpc urls`);
  }
  return new JsonRpcProvider(entry.rpcUrls[0]);
}
