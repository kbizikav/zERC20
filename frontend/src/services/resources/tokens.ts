import { ungzip } from 'pako';
import { normalizeTokensFile, TokensFile } from '@zerc20/sdk';
import type { NormalizedTokens } from '@/types/app';

const tokenCache = new Map<string, Promise<NormalizedTokens>>();

function asNormalizedTokens(file: TokensFile): NormalizedTokens {
  const normalized = normalizeTokensFile({ ...file, tokens: [...file.tokens] });
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

  if (typeof atob === 'function') {
    const binary = atob(normalized);
    const bytes = new Uint8Array(binary.length);
    for (let idx = 0; idx < binary.length; idx += 1) {
      bytes[idx] = binary.charCodeAt(idx);
    }
    return bytes;
  }

  const bufferCtor = (globalThis as { Buffer?: { from(data: string, format: 'base64'): Uint8Array } }).Buffer;
  if (bufferCtor) {
    return bufferCtor.from(normalized, 'base64');
  }

  throw new Error('Base64 decoding is not supported in this environment');
}

function loadEmbeddedTokens(compressed: string): Promise<NormalizedTokens> {
  return Promise.resolve()
    .then(() => {
      const gzippedBytes = base64ToBytes(compressed);
      const decompressed = ungzip(gzippedBytes);
      const text = new TextDecoder().decode(decompressed);
      try {
        return JSON.parse(text) as TokensFile;
      } catch (error) {
        throw new Error('Decompressed tokens blob is not valid JSON');
      }
    })
    .then(asNormalizedTokens);
}

export function loadTokens(compressed: string): Promise<NormalizedTokens> {
  const key = `compressed:${compressed}`;
  if (!tokenCache.has(key)) {
    tokenCache.set(key, loadEmbeddedTokens(compressed));
  }
  return tokenCache.get(key) as Promise<NormalizedTokens>;
}
