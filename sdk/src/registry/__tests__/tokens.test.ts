import { describe, expect, it } from 'vitest';

import { hasEnvVars, normalizeTokens, type TokensFile } from '../tokens.js';

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
