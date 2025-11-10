import { createWithdrawNovaWasm, WithdrawNovaWasm } from '../wasm/index.js';
import { normalizeHex } from '../utils/hex.js';
import type { IndexedEvent } from '../indexer/index.js';
import {
  appendDummySteps,
  formatFieldElement,
  toFieldHex,
  toLeafIndexString,
} from '../zkp/proofUtils.js';
import { hexToBytes } from '../utils/hex.js';
import type { NovaProverInput, NovaProverOutput } from '../types.js';

export async function runNovaProver(params: NovaProverInput): Promise<NovaProverOutput> {
  if (params.proofs.length !== params.events.length) {
    throw new Error('Events length must match proofs length for batch teleport');
  }
  // Ensure witnesses respect the circuit's ordering constraint on leaf indices.
  const proofEventPairs = params.events.map((event, idx) => ({
    event,
    proof: params.proofs[idx],
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
  const sortedEvents = proofEventPairs.map((pair) => pair.event);
  const sortedProofs = proofEventPairs.map((pair) => pair.proof);
  const wasm: WithdrawNovaWasm = await createWithdrawNovaWasm(
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
