import type { AggregationTreeState, BatchTeleportProof } from '../types.js';

export function getTeleportProofIndex(
  scope: AggregationTreeState['scope'],
  proof: BatchTeleportProof,
  idx?: number,
): bigint {
  if (scope === 'local') {
    if (proof.kind !== 'local') {
      throw new Error(indexError('local', idx));
    }
    return proof.treeIndex;
  }
  if (proof.kind !== 'global') {
    throw new Error(indexError('global', idx));
  }
  return proof.leafIndex;
}

function indexError(kind: 'local' | 'global', idx?: number): string {
  if (idx === undefined) {
    return `expected ${kind} teleport proof`;
  }
  return `proofs[${idx}] must be a ${kind} teleport proof`;
}
