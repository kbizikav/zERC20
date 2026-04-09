// SPDX-License-Identifier: BUSL-1.1

import { createPublicClient, http, type PublicClient } from 'viem';
import { decode as decodeBase64 } from 'base64-arraybuffer';
import { ungzip } from 'pako';

import { normalizeHex, toBigInt } from '../utils/hex.js';

/**
 * Function type for providing environment variable values.
 * Returns undefined if the variable is not set.
 */
export type EnvProvider = (key: string) => string | undefined;

const ENV_VAR_PATTERN = /\$\{([^}]+)\}/g;
const ENV_VAR_TEST = /\$\{[^}]+\}/;

/**
 * Expands environment variables in a string using the ${VAR_NAME} syntax.
 * @param value - The string potentially containing ${VAR_NAME} placeholders
 * @param envProvider - Function to resolve environment variable values
 * @returns The string with all environment variables expanded
 * @throws Error if an environment variable is referenced but not defined
 */
export function expandEnvVars(value: string, envProvider: EnvProvider): string {
  return value.replace(ENV_VAR_PATTERN, (match, varName: string) => {
    const envValue = envProvider(varName);
    if (envValue === undefined) {
      throw new Error(`Environment variable '${varName}' is not defined`);
    }
    return envValue;
  });
}

/**
 * Checks if a string contains environment variable placeholders.
 */
export function hasEnvVars(value: string): boolean {
  return ENV_VAR_TEST.test(value);
}

export interface TokenEntry {
  label: string;
  tokenAddress: string;
  verifierAddress: string;
  liquidityManagerAddress?: string;
  adaptorAddress?: string;
  chainId: bigint;
  deployedBlockNumber: bigint;
  rpcUrls: string[];
  legacyTx: boolean;
}

export interface HubEntry {
  hubAddress: string;
  chainId: bigint;
  rpcUrls: string[];
  legacyTx: boolean;
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

function normalizeRpcUrls(urls: unknown, label: string, envProvider?: EnvProvider): string[] {
  if (!Array.isArray(urls) || urls.length === 0) {
    throw new Error(`${label} must configure at least one rpc url`);
  }
  return urls.map((url) => {
    if (typeof url !== 'string' || url.trim().length === 0) {
      throw new Error(`${label} contains invalid rpc url`);
    }
    let trimmed = url.trim();
    if (envProvider && hasEnvVars(trimmed)) {
      trimmed = expandEnvVars(trimmed, envProvider);
    }
    return trimmed;
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

export function normalizeTokensFile(file: TokensFile, envProvider?: EnvProvider): TokensFile {
  if (!Array.isArray(file.tokens) || file.tokens.length === 0) {
    throw new Error('tokens array must be non-empty');
  }
  file.tokens = file.tokens.map((entry) => {
    const record = expectRecord(entry, 'token entry');
    const label = getStringField(record, ['label'], 'token label');
    const liquidityManagerAddressValue = getOptionalStringField(
      record,
      ['liquidityManagerAddress', 'liquidity_manager_address', 'minterAddress', 'minter_address'],
      `${label}.liquidityManagerAddress`,
    );
    const adaptorAddressValue = getOptionalStringField(
      record,
      ['adaptorAddress', 'adaptor_address'],
      `${label}.adaptorAddress`,
    );
    return {
      label,
      tokenAddress: toHexAddress(
        getStringField(record, ['tokenAddress', 'token_address'], `${label}.tokenAddress`),
      ),
      verifierAddress: toHexAddress(
        getStringField(record, ['verifierAddress', 'verifier_address'], `${label}.verifierAddress`),
      ),
      liquidityManagerAddress: liquidityManagerAddressValue ? toHexAddress(liquidityManagerAddressValue) : undefined,
      adaptorAddress: adaptorAddressValue ? toHexAddress(adaptorAddressValue) : undefined,
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
        envProvider,
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
      rpcUrls: normalizeRpcUrls(getValueField<unknown>(record, ['rpcUrls', 'rpc_urls'], 'hub.rpcUrls'), 'hub', envProvider),
      legacyTx: getOptionalBooleanField(record, ['legacyTx', 'legacy_tx'], 'hub.legacyTx') ?? false,
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

function asNormalizedTokens(file: TokensFile, envProvider?: EnvProvider): NormalizedTokens {
  const normalized = normalizeTokensFile(cloneTokensFile(file), envProvider);
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
  /**
   * Optional function to resolve environment variables in rpcUrls.
   * If provided, placeholders like ${VAR_NAME} will be expanded.
   */
  envProvider?: EnvProvider;
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

export function normalizeTokens(file: TokensFile, envProvider?: EnvProvider): NormalizedTokens {
  return asNormalizedTokens(file, envProvider);
}

export function clearTokensCache(cache?: Map<string, Promise<NormalizedTokens>>, key?: string): void {
  const target = cache ?? tokenCache;
  if (key) {
    target.delete(key);
    return;
  }
  target.clear();
}

function parseCompressed(compressed: string, options: LoadTokensOptions): NormalizedTokens {
  const gzippedBytes = base64ToBytes(compressed);
  const decompress = options.decompress ?? ((data: Uint8Array) => ungzip(data));
  const decompressed = decompress(gzippedBytes);
  const normalizedBytes =
    decompressed instanceof Uint8Array ? decompressed : new Uint8Array(decompressed as ArrayBufferLike);
  const parsed = decodeTokensPayload(normalizedBytes, options.decoder);
  return asNormalizedTokens(parsed, options.envProvider);
}

export function loadTokensFromCompressed(
  compressed: string,
  options: LoadTokensOptions = {},
): Promise<NormalizedTokens> {
  // When envProvider is set without an explicit cacheKey, bypass caching.
  // The envProvider function expands ${VAR} placeholders in RPC URLs at
  // parse time, so the output depends on runtime environment state.
  // Caching such results would silently return stale URLs when a different
  // envProvider (or different env values) is used in a subsequent call.
  if (options.envProvider && options.cacheKey == null) {
    return Promise.resolve().then(() => parseCompressed(compressed, options));
  }

  const cache = options.cache ?? tokenCache;
  const cacheKey = options.cacheKey ?? `compressed:${compressed}`;
  if (cache.has(cacheKey)) {
    return cache.get(cacheKey) as Promise<NormalizedTokens>;
  }

  const promise = Promise.resolve().then(() => parseCompressed(compressed, options));

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

export function createProviderForToken(entry: TokenEntry): PublicClient {
  if (entry.rpcUrls.length === 0) {
    throw new Error(`token '${entry.label}' is missing rpc urls`);
  }
  return createPublicClient({
    transport: http(entry.rpcUrls[0]),
  });
}
