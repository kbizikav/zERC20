import { describe, expect, it, beforeEach, vi } from 'vitest';
import { GLOBAL_TRANSFER_TREE_HEIGHT } from '../../constants.js';
import type { GlobalTeleportProofWithEvent } from '../../types.js';

const proveMock = vi.fn();
const createSingleWithdrawWasmMock = vi.fn(async () => ({
  prove: proveMock,
}));

vi.mock('../../wasm/index.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../wasm/index.js')>();
  return {
    ...actual,
    createSingleWithdrawWasm: createSingleWithdrawWasmMock,
  };
});

// Import after mocks
const { generateSingleTeleportProof } = await import('../receive.js');

function hexString(value: number): string {
  const hex = value.toString(16);
  return `0x${hex.length % 2 === 0 ? hex : `0${hex}`}`;
}

function padHex(value: number): string {
  return `0x${value.toString(16).padStart(64, '0')}`;
}

describe('generateSingleTeleportProof', () => {
  beforeEach(() => {
    proveMock.mockReset();
    createSingleWithdrawWasmMock.mockClear();
  });

  it('sends camelCase witness fields to the wasm prover', async () => {
    const proofResponse = {
      proofCalldata: hexString(123),
      publicInputs: [hexString(1), hexString(2)],
      treeDepth: GLOBAL_TRANSFER_TREE_HEIGHT,
    };
    proveMock.mockResolvedValue(proofResponse);

    const siblings = Array.from({ length: GLOBAL_TRANSFER_TREE_HEIGHT }, () => hexString(0));
    const globalProof: GlobalTeleportProofWithEvent = {
      event: {
        eventIndex: 0n,
        from: hexString(11),
        to: hexString(12),
        value: 5n,
        ethBlockNumber: 42n,
      },
      siblings,
      leafIndex: 0n,
    };

    const result = await generateSingleTeleportProof({
      wasmArtifacts: {
        localPk: new Uint8Array(),
        localVk: new Uint8Array(),
        globalPk: new Uint8Array(),
        globalVk: new Uint8Array(),
      },
      aggregationState: {
        latestAggSeq: 1n,
        aggregationRoot: padHex(99),
        snapshot: [],
        transferTreeIndices: [],
        chainIds: [],
      },
      recipientFr: padHex(7),
      secretHex: padHex(8),
      event: globalProof.event,
      proof: globalProof,
    });

    expect(result).toEqual(proofResponse);
    expect(proveMock).toHaveBeenCalledTimes(1);

    const witnessArg = proveMock.mock.calls[0]?.[0];
    expect(witnessArg).toBeDefined();
    expect(witnessArg).toHaveProperty('merkleRoot', padHex(99));
    expect(witnessArg).toHaveProperty('withdrawValue');
    expect(witnessArg.withdrawValue).toMatch(/^0x[0-9a-f]{64}$/i);
    expect(witnessArg).not.toHaveProperty('merkle_root');
    expect(witnessArg).not.toHaveProperty('withdraw_value');
  });
});
