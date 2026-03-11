import { afterEach, describe, expect, it, vi } from 'vitest';
import { decodeFunctionData, type Address, type Hex, type PublicClient } from 'viem';

import { gelatoRelayAbi } from '../abi.js';
import {
  encodeRelaySingleTeleport,
  encodeRelayTeleport,
  encodeRelayTransfer,
  encodeRelayUnwrap,
  estimateRelayerFee,
  submitTeleportRelay,
  waitForRelayTask,
} from '../gelatoRelay.js';
import { RelayTaskState } from '../types.js';

const getEstimatedFeeMock = vi.fn();
const callWithSyncFeeMock = vi.fn();
const getTaskStatusMock = vi.fn();

vi.mock('@gelatonetwork/relay-sdk', () => ({
  GelatoRelay: class {
    getEstimatedFee(...args: unknown[]) {
      return getEstimatedFeeMock(...args);
    }

    callWithSyncFee(...args: unknown[]) {
      return callWithSyncFeeMock(...args);
    }

    getTaskStatus(...args: unknown[]) {
      return getTaskStatusMock(...args);
    }
  },
}));

function bytes32Hex(value: number): Hex {
  return `0x${value.toString(16).padStart(64, '0')}`;
}

describe('relay helpers', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    getEstimatedFeeMock.mockReset();
    callWithSyncFeeMock.mockReset();
    getTaskStatusMock.mockReset();
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

  it('encodes relayTeleport with the max gelato fee argument', () => {
    const data = encodeRelayTeleport({
      isGlobal: true,
      rootHint: 9n,
      gr: {
        chainId: 1n,
        recipient: bytes32Hex(0x11),
        tweak: bytes32Hex(0x22),
      },
      proof: '0x1234' as Hex,
      feeAuth: {
        relayerFee: 3n,
        maxFee: 4n,
        deadline: 5n,
        signature: '0xabcd' as Hex,
      },
      maxGelatoFee: 6n,
    });

    const decoded = decodeFunctionData({ abi: gelatoRelayAbi, data });
    expect(decoded.functionName).toBe('relayTeleport');
    expect(decoded.args?.[5]).toBe(6n);
  });

  it('encodes relaySingleTeleport with the max gelato fee argument', () => {
    const data = encodeRelaySingleTeleport({
      isGlobal: false,
      rootHint: 2n,
      gr: {
        chainId: 1n,
        recipient: bytes32Hex(0x11),
        tweak: bytes32Hex(0x22),
      },
      proof: '0x1234' as Hex,
      feeAuth: {
        relayerFee: 3n,
        maxFee: 4n,
        deadline: 5n,
        signature: '0xabcd' as Hex,
      },
      maxGelatoFee: 7n,
    });

    const decoded = decodeFunctionData({ abi: gelatoRelayAbi, data });
    expect(decoded.functionName).toBe('relaySingleTeleport');
    expect(decoded.args?.[5]).toBe(7n);
  });

  it('encodes relayTransfer with the relayer fee argument', () => {
    const data = encodeRelayTransfer({
      owner: '0x0000000000000000000000000000000000000001' as Address,
      to: '0x0000000000000000000000000000000000000002' as Address,
      amount: 100n,
      relayerFee: 9n,
      maxGelatoFee: 12n,
      deadline: 123n,
      permitSig: '0x1234' as Hex,
      relaySig: '0xabcd' as Hex,
    });

    const decoded = decodeFunctionData({ abi: gelatoRelayAbi, data });
    expect(decoded.functionName).toBe('relayTransfer');
    expect(decoded.args?.[3]).toBe(9n);
    expect(decoded.args?.[4]).toBe(12n);
  });

  it('returns buffered relayer and max gelato fees', async () => {
    getEstimatedFeeMock.mockResolvedValue(100n);
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

  it('submits relay requests with Gelato overhead added to gasLimit', async () => {
    callWithSyncFeeMock.mockResolvedValue({ taskId: 'task-1' });

    const result = await submitTeleportRelay({
      chainId: 1,
      gelatoRelayAddress: '0x0000000000000000000000000000000000000004' as Address,
      feeToken: '0x0000000000000000000000000000000000000003' as Address,
      calldata: '0x1234' as Hex,
      gasLimit: 500_000n,
    });

    expect(result).toEqual({ taskId: 'task-1' });
    expect(callWithSyncFeeMock).toHaveBeenCalledWith(
      expect.objectContaining({
        chainId: 1n,
        target: '0x0000000000000000000000000000000000000004',
        data: '0x1234',
      }),
      { gasLimit: 650_000n },
      undefined,
    );
  });

  it('returns terminal relay task states without unsafe casting', async () => {
    getTaskStatusMock.mockResolvedValue({
      taskId: 'task-2',
      taskState: 'ExecSuccess',
      transactionHash: '0xabc',
      blockNumber: 1,
      executionDate: '2026-03-10T00:00:00Z',
      lastCheckMessage: 'ok',
    } as never);

    const result = await waitForRelayTask('task-2', { polls: 1, intervalMs: 0 });
    expect(result.taskState).toBe(RelayTaskState.ExecSuccess);
    expect(result.transactionHash).toBe('0xabc');
  });

  it('throws on unknown Gelato task states', async () => {
    getTaskStatusMock.mockResolvedValue({
      taskId: 'task-3',
      taskState: 'BrandNewState',
    } as never);

    await expect(waitForRelayTask('task-3', { polls: 1, intervalMs: 0 })).rejects.toThrow(
      /unknown Gelato task state/,
    );
  });
});
