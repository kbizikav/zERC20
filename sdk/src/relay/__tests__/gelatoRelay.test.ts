import { GelatoRelay } from '@gelatonetwork/relay-sdk';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { decodeFunctionData, type Address, type Hex, type PublicClient } from 'viem';

import { gelatoRelayAbi } from '../abi.js';
import { encodeRelayUnwrap, estimateRelayerFee } from '../gelatoRelay.js';

describe('relay helpers', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('encodes relayUnwrap with the relayer fee argument', () => {
    const owner = '0x0000000000000000000000000000000000000001' as Address;
    const receiver = '0x0000000000000000000000000000000000000002' as Address;
    const data = encodeRelayUnwrap({
      owner,
      amount: 100n,
      receiver,
      relayerFee: 7n,
      maxGelatoFee: 11n,
      deadline: 123n,
      permitSig: '0x1234' as Hex,
      relaySig: '0xabcd' as Hex,
    });

    const decoded = decodeFunctionData({
      abi: gelatoRelayAbi,
      data,
    });

    expect(decoded.functionName).toBe('relayUnwrap');
    expect(decoded.args?.[3]).toBe(7n);
    expect(decoded.args?.[4]).toBe(11n);
  });

  it('returns buffered relayer and max gelato fees', async () => {
    vi.spyOn(GelatoRelay.prototype, 'getEstimatedFee').mockResolvedValue(100n);
    const publicClient = {
      readContract: vi.fn().mockResolvedValue(10n),
    } as unknown as PublicClient;

    const result = await estimateRelayerFee({
      chainId: 1,
      feeToken: '0x0000000000000000000000000000000000000003' as Address,
      gasLimit: 1_000_000n,
      liquidityManagerAddress: '0x0000000000000000000000000000000000000004' as Address,
      publicClient,
    });

    expect(result).toEqual({
      relayerFee: 116n,
      gelatoFee: 100n,
      maxGelatoFee: 105n,
      unwrapFee: 10n,
    });
  });
});
