import { describe, expect, it, beforeEach, vi } from 'vitest';
import { GLOBAL_TRANSFER_TREE_HEIGHT } from '../../constants.js';
import type { GlobalTeleportProofWithEvent, LocalTeleportProof } from '../../types.js';

const proveMock = vi.fn();
const runNovaProverMock = vi.fn();
const createSingleWithdrawWasmMock = vi.fn(async () => ({
  prove: proveMock,
}));
const loadSingleTeleportArtifactsMock = vi.fn(async () => ({
  localPk: new Uint8Array(),
  localVk: new Uint8Array(),
  globalPk: new Uint8Array(),
  globalVk: new Uint8Array(),
}));

vi.mock('../../wasm/index.js', () => ({
  createSingleWithdrawWasm: createSingleWithdrawWasmMock,
  getDefaultWasmRuntime: vi.fn(() => ({})),
  fetchAggregationTreeState: vi.fn(),
  fetchTransferEvents: vi.fn(),
  separateEventsByEligibility: vi.fn(),
  fetchLocalTeleportMerkleProofs: vi.fn(),
  generateGlobalTeleportMerkleProofs: vi.fn(),
}));

vi.mock('../../wasm/artifacts.js', () => ({
  loadSingleTeleportArtifacts: loadSingleTeleportArtifactsMock,
}));

vi.mock('../../zkp/proofService.js', () => ({
  ProofService: class {
    async runNovaProver(...args: unknown[]) {
      return runNovaProverMock(...args);
    }
  },
}));

// Import after mocks
const { generateBatchTeleportProof, generateSingleTeleportProof } = await import('../receive.js');

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
    runNovaProverMock.mockReset();
    createSingleWithdrawWasmMock.mockClear();
    loadSingleTeleportArtifactsMock.mockClear();
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
      aggregationState: {
        scope: 'global',
        rootHint: 1n,
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
    expect(loadSingleTeleportArtifactsMock).toHaveBeenCalledTimes(1);

    const witnessArg = proveMock.mock.calls[0]?.[0];
    expect(witnessArg).toBeDefined();
    expect(witnessArg).toHaveProperty('merkleRoot', padHex(99));
    expect(witnessArg).toHaveProperty('withdrawValue');
    expect(witnessArg.withdrawValue).toMatch(/^0x[0-9a-f]{64}$/i);
    expect(witnessArg).toHaveProperty('from', padHex(11));
    expect(witnessArg).not.toHaveProperty('merkle_root');
    expect(witnessArg).not.toHaveProperty('withdraw_value');
  });

  it('uses the local decider circuit for local aggregation roots', async () => {
    runNovaProverMock.mockResolvedValue({
      ivcProof: new Uint8Array([1, 2, 3]),
      finalState: [],
      steps: 1,
    });
    const produceDeciderProof = vi.fn().mockResolvedValue(new Uint8Array([4, 5, 6]));
    const localProof: LocalTeleportProof = {
      treeIndex: 3n,
      event: {
        eventIndex: 0n,
        from: hexString(11),
        to: hexString(12),
        value: 5n,
        ethBlockNumber: 42n,
      },
      siblings: [],
    };

    const result = await generateBatchTeleportProof({
      aggregationState: {
        scope: 'local',
        rootHint: 1n,
        aggregationRoot: padHex(99),
        snapshot: [],
        transferTreeIndices: [],
        chainIds: [],
      },
      recipientFr: padHex(7),
      secretHex: padHex(8),
      proofs: [localProof],
      events: [localProof.event],
      decider: { produceDeciderProof } as never,
      offloadToWorker: false,
    });

    expect(runNovaProverMock).toHaveBeenCalledWith(
      expect.objectContaining({
        proofs: [localProof],
        events: [localProof.event],
      }),
      { offloadToWorker: false },
    );
    expect(produceDeciderProof).toHaveBeenCalledWith('withdraw_local', new Uint8Array([1, 2, 3]));
    expect(result.deciderProof).toEqual(new Uint8Array([4, 5, 6]));
  });

  it('uses the global decider circuit for global aggregation roots', async () => {
    runNovaProverMock.mockResolvedValue({
      ivcProof: new Uint8Array([7, 8, 9]),
      finalState: [],
      steps: 1,
    });
    const produceDeciderProof = vi.fn().mockResolvedValue(new Uint8Array([10, 11, 12]));

    await generateBatchTeleportProof({
      aggregationState: {
        scope: 'global',
        rootHint: 1n,
        aggregationRoot: padHex(99),
        snapshot: [],
        transferTreeIndices: [],
        chainIds: [],
      },
      recipientFr: padHex(7),
      secretHex: padHex(8),
      proofs: [],
      events: [],
      decider: { produceDeciderProof } as never,
      offloadToWorker: false,
    });

    expect(produceDeciderProof).toHaveBeenCalledWith('withdraw_global', new Uint8Array([7, 8, 9]));
  });
});
