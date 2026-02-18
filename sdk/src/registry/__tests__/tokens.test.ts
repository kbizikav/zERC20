import { describe, expect, it, beforeEach } from 'vitest';

import type { EnvProvider, NormalizedTokens, TokensFile } from '../tokens.js';
import { hasEnvVars, normalizeTokens, loadTokensFromCompressed, clearTokensCache } from '../tokens.js';

describe('hasEnvVars', () => {
  it('returns true for consecutive env-var strings', () => {
    expect(hasEnvVars('https://${RPC_A}/v2')).toBe(true);
    expect(hasEnvVars('https://${RPC_B}/v2')).toBe(true);
    expect(hasEnvVars('https://fixed.example')).toBe(false);
  });
});

describe('normalizeTokens env expansion', () => {
  it('expands all placeholders across multiple rpc urls', () => {
    const file: TokensFile = {
      tokens: [
        {
          label: 'test-token',
          tokenAddress: '0x1111111111111111111111111111111111111111',
          verifierAddress: '0x2222222222222222222222222222222222222222',
          chainId: 1n,
          deployedBlockNumber: 123n,
          rpcUrls: [
            'https://${ALCHEMY_KEY}.alchemy.example',
            'https://rpc.example/${INFURA_KEY}',
          ],
          legacyTx: false,
        },
      ],
      hub: {
        hubAddress: '0x3333333333333333333333333333333333333333',
        chainId: 1n,
        rpcUrls: [
          'https://${ALCHEMY_KEY}.hub.example',
          'https://hub.example/${INFURA_KEY}',
        ],
        legacyTx: false,
      },
    };

    const envProvider = (key: string): string | undefined => {
      if (key === 'ALCHEMY_KEY') return 'alchemy-value';
      if (key === 'INFURA_KEY') return 'infura-value';
      return undefined;
    };

    const normalized = normalizeTokens(file, envProvider);

    expect(normalized.tokens[0].rpcUrls).toEqual([
      'https://alchemy-value.alchemy.example',
      'https://rpc.example/infura-value',
    ]);
    expect(normalized.hub?.rpcUrls).toEqual([
      'https://alchemy-value.hub.example',
      'https://hub.example/infura-value',
    ]);
  });
});

// Base64-encoded gzip of a tokens JSON with rpcUrls containing ${TEST_RPC_URL}
const COMPRESSED_WITH_ENV =
  'H4sIAAAAAAAAE52PQQrCMBRE7zK4zCK6zE7FhSAiNV1JKWnyq6XfpiRVWkrvLsELiLOcxxuYGYNvqYtQtxlsKmIoaIqDTjXEF2+dCxQjFOQof8saAm8KTd1Q+EPfQMA+TNMdHRTSmKOe/URux96259ezopCIlBAIvc0DpxNYzfpw1WV22Zd5dlpQCDDdjZ30CFUbjrQUywfm3NPl9gAAAA==';

// Base64-encoded gzip of a tokens JSON with plain rpcUrls (no env vars)
const COMPRESSED_PLAIN =
  'H4sIAAAAAAAAE52PsQ6CMBRF/+XODVTHbrq5OOFkGEr7FMKDNq/VQAj/bhp/wHjHe3JucjfkMNKcYO4b2HbEMGgo5abUUF988l4oJRjoRf+WAxTeJMNjIPlDP0LB9XaYLx4GZcxT5LCSP3Nw4/U1dSSFaA0Fie4mXE6gzzkmU9cSXUWLnSJT5cKEVoHpad3aLDAPy4n2dv8AMSZ4Gf4AAAA=';

describe('loadTokensFromCompressed', () => {
  let cache: Map<string, Promise<NormalizedTokens>>;

  beforeEach(() => {
    cache = new Map();
    clearTokensCache();
  });

  it('caches results when no envProvider is given', async () => {
    const first = await loadTokensFromCompressed(COMPRESSED_PLAIN, { cache });
    const second = await loadTokensFromCompressed(COMPRESSED_PLAIN, { cache });

    expect(first).toBe(second); // same reference — served from cache
    expect(cache.size).toBe(1);
  });

  it('bypasses cache when envProvider is set without cacheKey', async () => {
    const envA: EnvProvider = (key) => (key === 'TEST_RPC_URL' ? 'https://rpc-a.example.com' : undefined);
    const envB: EnvProvider = (key) => (key === 'TEST_RPC_URL' ? 'https://rpc-b.example.com' : undefined);

    const resultA = await loadTokensFromCompressed(COMPRESSED_WITH_ENV, { cache, envProvider: envA });
    const resultB = await loadTokensFromCompressed(COMPRESSED_WITH_ENV, { cache, envProvider: envB });

    // Each call should produce independently resolved URLs
    expect(resultA.tokens[0].rpcUrls[0]).toBe('https://rpc-a.example.com');
    expect(resultB.tokens[0].rpcUrls[0]).toBe('https://rpc-b.example.com');

    // Cache should NOT have been populated
    expect(cache.size).toBe(0);
  });

  it('uses cache when envProvider is set WITH explicit cacheKey', async () => {
    const envA: EnvProvider = (key) => (key === 'TEST_RPC_URL' ? 'https://rpc-a.example.com' : undefined);
    const envB: EnvProvider = (key) => (key === 'TEST_RPC_URL' ? 'https://rpc-b.example.com' : undefined);

    const resultA = await loadTokensFromCompressed(COMPRESSED_WITH_ENV, {
      cache,
      envProvider: envA,
      cacheKey: 'shared-key',
    });
    const resultB = await loadTokensFromCompressed(COMPRESSED_WITH_ENV, {
      cache,
      envProvider: envB,
      cacheKey: 'shared-key',
    });

    // Second call should return the cached result from envA (same reference)
    expect(resultA).toBe(resultB);
    expect(resultA.tokens[0].rpcUrls[0]).toBe('https://rpc-a.example.com');
    expect(cache.size).toBe(1);
  });

  it('does not leak envProvider results into plain cache', async () => {
    const env: EnvProvider = (key) => (key === 'TEST_RPC_URL' ? 'https://rpc-env.example.com' : undefined);

    // Call with envProvider (should bypass cache)
    await loadTokensFromCompressed(COMPRESSED_WITH_ENV, { cache, envProvider: env });

    // Call without envProvider using different payload (should use cache)
    const plain = await loadTokensFromCompressed(COMPRESSED_PLAIN, { cache });

    expect(cache.size).toBe(1);
    expect(plain.tokens[0].rpcUrls[0]).toBe('https://rpc.example.com');
  });
});
