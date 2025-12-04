import type { AggregationTreeState } from '../types.js';

export function getTreeIndexForChain(state: AggregationTreeState, chainId: bigint): bigint {
  const position = state.chainIds.findIndex((id) => id === chainId);
  if (position === -1) {
    throw new Error(`Chain id ${chainId} not found in aggregation state`);
  }
  if (position >= state.transferTreeIndices.length) {
    throw new Error(`Aggregation snapshot missing tree index for chain ${chainId}`);
  }
  return state.transferTreeIndices[position];
}
