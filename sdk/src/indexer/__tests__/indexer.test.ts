import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { HttpIndexerClient } from '../index.js';

const INDEXER_URL = 'http://indexer.example.com/';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const tokensConfig = JSON.parse(
  readFileSync(path.join(__dirname, 'fixtures', 'tokens.json'), 'utf8'),
) as {
  tokens: Array<{
    chain_id: number;
    token_address: string;
  }>;
};

const { chain_id: chainIdFromConfig, token_address: tokenAddressFromConfig } = tokensConfig.tokens[0];
const CHAIN_ID = BigInt(chainIdFromConfig);
const TOKEN_ADDRESS = tokenAddressFromConfig;
const TARGET_INDEX = 1n;
const LEAF_INDICES = [0n];

describe('HttpIndexerClient.proveMany', () => {
  it('serialises numeric proof payload fields as numbers', async () => {
    const fetchMock = vi.fn(async (_input: any, init?: any) => {
      expect(init).toBeDefined();
      expect(init?.method).toBe('POST');
      expect(init?.headers).toEqual({ 'Content-Type': 'application/json' });

      const rawBody = init?.body;
      expect(typeof rawBody).toBe('string');
      const payload = JSON.parse(String(rawBody));
      expect(payload).toEqual({
        chain_id: Number(CHAIN_ID),
        token_address: TOKEN_ADDRESS,
        target_index: Number(TARGET_INDEX),
        leaf_indices: LEAF_INDICES.map((idx) => Number(idx)),
      });

      return new Response('[]', {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    });

    const client = new HttpIndexerClient(INDEXER_URL, fetchMock as unknown as typeof fetch);
    const result = await client.proveMany({
      chainId: CHAIN_ID,
      tokenAddress: TOKEN_ADDRESS,
      targetIndex: TARGET_INDEX,
      leafIndices: LEAF_INDICES,
    });

    expect(result).toEqual([]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('surfaces backend error message details on non-2xx responses', async () => {
    const fetchMock = vi.fn(async () => {
      return new Response('leaf index 0 not present at tree index 1', {
        status: 400,
        headers: { 'Content-Type': 'text/plain' },
      });
    });

    const client = new HttpIndexerClient(INDEXER_URL, fetchMock as unknown as typeof fetch);

    await expect(
      client.proveMany({
        chainId: CHAIN_ID,
        tokenAddress: TOKEN_ADDRESS,
        targetIndex: TARGET_INDEX,
        leafIndices: LEAF_INDICES,
      }),
    ).rejects.toThrow('indexer proofs request failed with status 400: leaf index 0 not present at tree index 1');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('throws before sending the request when integers exceed the safe range', async () => {
    const fetchMock = vi.fn();
    const client = new HttpIndexerClient(INDEXER_URL, fetchMock as unknown as typeof fetch);

    await expect(
      client.proveMany({
        chainId: BigInt(Number.MAX_SAFE_INTEGER) + 1n,
        tokenAddress: TOKEN_ADDRESS,
        targetIndex: 1n,
        leafIndices: [0n],
      }),
    ).rejects.toThrow(/chainId .* exceeds the safe integer range/);

    expect(fetchMock).not.toHaveBeenCalled();
  });
});
