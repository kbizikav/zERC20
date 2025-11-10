import { hexToBytes, normalizeHex } from '../utils/hex.js';
import type {
  GlobalTeleportProof,
  IndexedEvent,
  NovaProverInput,
  NovaProverOutput,
  SingleTeleportArtifacts,
  SingleTeleportParams,
} from '../types.js';
import { appendDummySteps, formatFieldElement, toFieldHex, toLeafIndexString } from './proofUtils.js';
import { canUseZkpWorker, disableZkpWorker, runZkpWorkerJob } from './workerClient.js';
import type { WasmRuntime } from '../wasm/index.js';

export interface ProofServiceOptions {
  /** Disable automatic worker offloading. */
  defaultToWorker?: boolean;
}

export interface ProofRunOptions {
  /** Override worker usage for a single request. */
  offloadToWorker?: boolean;
}

export class ProofService {
  constructor(private readonly wasm: WasmRuntime, private readonly options: ProofServiceOptions = {}) {}

  async createSingleTeleportProof(
    params: SingleTeleportParams,
    runOptions?: ProofRunOptions,
  ): Promise<SingleTeleportArtifacts> {
    if (this.shouldUseWorker(runOptions)) {
      try {
        return await runZkpWorkerJob('singleTeleport', params);
      } catch (error) {
        console.warn('[zkERC20] Worker single proof failed; falling back to main thread.', error);
        disableZkpWorker(error);
      }
    }
    return this.computeSingleTeleportProof(params);
  }

  async runNovaProver(params: NovaProverInput, runOptions?: ProofRunOptions): Promise<NovaProverOutput> {
    if (this.shouldUseWorker(runOptions)) {
      try {
        return await runZkpWorkerJob('nova', params);
      } catch (error) {
        console.warn('[zkERC20] Worker nova prover failed; falling back to main thread.', error);
        disableZkpWorker(error);
      }
    }
    return this.computeNovaProof(params);
  }

  private shouldUseWorker(runOptions?: ProofRunOptions): boolean {
    if (runOptions?.offloadToWorker === false) {
      return false;
    }
    if (runOptions?.offloadToWorker === true) {
      return canUseZkpWorker();
    }
    if (this.options.defaultToWorker === false) {
      return false;
    }
    return canUseZkpWorker();
  }

  private async computeSingleTeleportProof(
    params: SingleTeleportParams,
  ): Promise<SingleTeleportArtifacts> {
    const wasm = await this.wasm.createSingleWithdrawProgram(
      params.wasmArtifacts.localPk,
      params.wasmArtifacts.localVk,
      params.wasmArtifacts.globalPk,
      params.wasmArtifacts.globalVk,
    );
    const zeroField = formatFieldElement('0x0', 'delta');
    const witness = {
      merkleRoot: formatFieldElement(params.aggregationState.aggregationRoot, 'merkleRoot'),
      recipient: formatFieldElement(params.recipientFr, 'recipient'),
      withdrawValue: formatFieldElement(toFieldHex(params.event.value), 'withdrawValue'),
      value: formatFieldElement(toFieldHex(params.event.value), 'value'),
      delta: zeroField,
      secret: formatFieldElement(params.secretHex, 'secret'),
      leafIndex: toLeafIndexString(params.proof.leafIndex),
      siblings: params.proof.siblings.map((sibling, idx) =>
        formatFieldElement(sibling, `proof.siblings[${idx}]`),
      ),
    };
    try {
      const result = await wasm.prove(witness);
      return {
        proofCalldata: normalizeHex(result.proofCalldata),
        publicInputs: result.publicInputs.map((input: string) => normalizeHex(input)),
        treeDepth: result.treeDepth,
      };
    } catch (error) {
      const debugWitness = {
        ...witness,
        secret: '[redacted]',
      };
      console.error('[zkERC20] createSingleTeleportProof failed', debugWitness, error);
      throw new Error(
        error instanceof Error
          ? `single teleport proof failed: ${error.message}`
          : `single teleport proof failed: ${String(error)}`,
      );
    }
  }

  private async computeNovaProof(params: NovaProverInput): Promise<NovaProverOutput> {
    if (params.proofs.length !== params.events.length) {
      throw new Error('events length must match proofs length for batch teleport');
    }

    const { events: sortedEvents, proofs: sortedProofs } = sortProofsByLeafIndex(
      params.events,
      params.proofs,
    );

    const wasm = await this.wasm.createWithdrawNovaProgram(
      params.wasmArtifacts.localPp,
      params.wasmArtifacts.localVp,
      params.wasmArtifacts.globalPp,
      params.wasmArtifacts.globalVp,
    );
    const z0 = [
      formatFieldElement(params.aggregationState.aggregationRoot, 'z0[0]'),
      formatFieldElement(params.recipientFr, 'z0[1]'),
      formatFieldElement('0x0', 'z0[2]'),
      formatFieldElement('0x0', 'z0[3]'),
    ];
    const steps = sortedEvents.map((event, idx) => ({
      is_dummy: false,
      value: formatFieldElement(toFieldHex(event.value), `events[${idx}].value`),
      secret: formatFieldElement(params.secretHex, `events[${idx}].secret`),
      leafIndex: toLeafIndexString(sortedProofs[idx].leafIndex),
      siblings: sortedProofs[idx].siblings.map((sibling, siblingIdx) =>
        formatFieldElement(sibling, `proofs[${idx}].siblings[${siblingIdx}]`),
      ),
    }));

    appendDummySteps(steps);

    try {
      const proofResult = await wasm.prove(z0, steps);
      const ivcProofBytes = hexToBytes(proofResult.ivcProof);
      return {
        ivcProof: ivcProofBytes,
        finalState: proofResult.finalState.map((value: string) => normalizeHex(value)),
        steps: proofResult.steps,
      };
    } catch (error) {
      const debugSiblings = steps.map((step) => ({
        isDummy: step.is_dummy,
        leafIndex: step.leafIndex,
        siblings: step.siblings.slice(0, 4),
      }));
      console.error(
        '[zkERC20] runNovaProver failed',
        { firstSteps: debugSiblings.slice(0, 2), totalSteps: steps.length },
        error,
      );
      throw new Error(
        error instanceof Error
          ? `batch teleport proof failed: ${error.message}`
          : `batch teleport proof failed: ${String(error)}`,
      );
    }
  }
}

function sortProofsByLeafIndex(
  events: readonly IndexedEvent[],
  proofs: readonly GlobalTeleportProof[],
): { events: IndexedEvent[]; proofs: GlobalTeleportProof[] } {
  const proofEventPairs = events.map((event, idx) => ({
    event,
    proof: proofs[idx],
  }));
  proofEventPairs.sort((a, b) => {
    if (a.proof.leafIndex < b.proof.leafIndex) {
      return -1;
    }
    if (a.proof.leafIndex > b.proof.leafIndex) {
      return 1;
    }
    return 0;
  });
  return {
    events: proofEventPairs.map((pair) => pair.event),
    proofs: proofEventPairs.map((pair) => pair.proof),
  };
}
