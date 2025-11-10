import { JsonRpcProvider } from 'ethers';

import { normalizeHex } from '../utils/hex.js';

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

type MaybeString = string | number | bigint;

function toHexAddress(value: string): string {
  return normalizeHex(value);
}

function toBigInt(value: MaybeString, label: string): bigint {
  if (typeof value === 'bigint') {
    return value;
  }
  if (typeof value === 'number') {
    return BigInt(value);
  }
  const trimmed = value.trim();
  if (trimmed.startsWith('0x') || trimmed.startsWith('0X')) {
    return BigInt(trimmed);
  }
  if (/^\d+$/.test(trimmed)) {
    return BigInt(trimmed);
  }
  throw new Error(`${label} must be a bigint-compatible value`);
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
      chainId: toBigInt(
        getValueField<MaybeString>(record, ['chainId', 'chain_id'], `${label}.chainId`),
        `${label}.chainId`,
      ),
      deployedBlockNumber: toBigInt(
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
      chainId: toBigInt(getValueField<MaybeString>(record, ['chainId', 'chain_id'], 'hub.chainId'), 'hub.chainId'),
      rpcUrls: normalizeRpcUrls(getValueField<unknown>(record, ['rpcUrls', 'rpc_urls'], 'hub.rpcUrls'), 'hub'),
    };
  }

  return file;
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

export function createProviderForHub(entry: HubEntry): JsonRpcProvider {
  if (entry.rpcUrls.length === 0) {
    throw new Error('hub configuration is missing rpc urls');
  }
  return new JsonRpcProvider(entry.rpcUrls[0]);
}
