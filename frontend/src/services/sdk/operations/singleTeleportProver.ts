import { createSingleWithdrawWasm, type SingleWithdrawWasm } from '../wasm/index.js';
import { normalizeHex } from '../core/utils.js';
import { formatFieldElement, toFieldHex, toLeafIndexString } from './proofUtils.js';
import type { AggregationTreeState, GlobalTeleportProof } from './teleport.js';
import type { IndexedEvent } from '../indexer/index.js';

export interface SingleTeleportArtifacts {
  proofCalldata: string;
  publicInputs: string[];
  treeDepth: number;
}

export interface SingleTeleportProverInput {
  wasmArtifacts: {
    localPk: Uint8Array;
    localVk: Uint8Array;
    globalPk: Uint8Array;
    globalVk: Uint8Array;
  };
  aggregationState: AggregationTreeState;
  recipientFr: string;
  secretHex: string;
  event: IndexedEvent;
  proof: GlobalTeleportProof;
}

export async function runSingleTeleportProver(
  params: SingleTeleportProverInput,
): Promise<SingleTeleportArtifacts> {
  const wasm: SingleWithdrawWasm = await createSingleWithdrawWasm(
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
    console.error('[zkERC20] generateSingleTeleportProof failed', debugWitness, error);
    throw new Error(
      error instanceof Error
        ? `single teleport proof failed: ${error.message}`
        : `single teleport proof failed: ${String(error)}`,
    );
  }
}
