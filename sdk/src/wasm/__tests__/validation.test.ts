import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { WasmRuntime } from '../index.js';
import { NUM_BATCH_INVOICES } from '../../constants.js';
import type { FetchAggregationTreeStateParams } from '../../types.js';

describe('WasmRuntime input validation', () => {
  let runtime: WasmRuntime;

  beforeEach(() => {
    runtime = new WasmRuntime();
    vi.spyOn(runtime as unknown as { ensureReady(): Promise<void> }, 'ensureReady').mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('deriveInvoiceBatch rejects out-of-range subId', async () => {
    await expect(
      runtime.deriveInvoiceBatch('0x1', '0x2', NUM_BATCH_INVOICES, 0n, '0x3'),
    ).rejects.toThrow(/subId must be/);
    await expect(
      runtime.deriveInvoiceBatch('0x1', '0x2', -1, 0n, '0x3'),
    ).rejects.toThrow(/subId must be/);
  });

  test('fetchAggregationTreeState rejects negative eventBlockSpan numbers', async () => {
    const params: FetchAggregationTreeStateParams = {
      eventBlockSpan: -1,
      hub: {
        hubAddress: '0x01',
        chainId: 1n,
        rpcUrls: ['https://example.com'],
      },
      token: {
        label: 'token',
        tokenAddress: '0x02',
        verifierAddress: '0x03',
        chainId: 1n,
        deployedBlockNumber: 0n,
        rpcUrls: ['https://example.com'],
      },
    };

    await expect(runtime.fetchAggregationTreeState(params)).rejects.toThrow(/eventBlockSpan/);
  });
});
