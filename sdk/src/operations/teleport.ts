import { Contract, EventLog, JsonRpcProvider, zeroPadValue } from 'ethers';

import {
  AGGREGATION_TREE_HEIGHT,
  GLOBAL_TRANSFER_TREE_HEIGHT,
  TRANSFER_TREE_HEIGHT,
} from '../constants.js';
import { HistoricalProof, IndexedEvent } from '../indexer/index.js';
import { aggregationMerkleProof, aggregationRoot } from '../wasm/index.js';
import { normalizeHex } from '../utils/hex.js';
import type {
  AggregationTreeState,
  EventsWithEligibility,
  GlobalTeleportProofWithEvent,
} from '../types.js';

export type { AggregationTreeState, EventsWithEligibility, GlobalTeleportProofWithEvent };

export interface AggregationStateParams {
  hubContract: Contract;
  verifierContract: Contract;
  provider: JsonRpcProvider;
  eventBlockSpan?: bigint;
}

export async function fetchAggregationTreeState(params: AggregationStateParams): Promise<AggregationTreeState> {
  const blockSpan = params.eventBlockSpan ?? 5_000n;
  const latestAggSeq = BigInt(await params.verifierContract.latestAggSeq());
  if (latestAggSeq === 0n) {
    throw new Error('No aggregation reached yet (latestAggSeq is zero)');
  }

  const onchainRoot = zeroPadValue(
    normalizeHex(await params.verifierContract.globalTransferRoots(latestAggSeq)),
    32,
  );

  const provider = params.provider;
  let toBlock = BigInt(await provider.getBlockNumber());
  let matchingEvent: EventLog | null = null;
  while (toBlock >= 0n) {
    const fromBlock = toBlock + 1n > blockSpan ? toBlock + 1n - blockSpan : 0n;
    const events = await params.hubContract.queryFilter(
      params.hubContract.filters.AggregationRootUpdated(),
      fromBlock,
      toBlock,
    );
    for (const event of events.reverse()) {
      if (!('args' in event) || !event.args) {
        continue;
      }
      const aggSeq = BigInt(event.args.aggSeq ?? 0);
      if (aggSeq === latestAggSeq) {
        matchingEvent = event as EventLog;
        break;
      }
    }
    if (matchingEvent) {
      break;
    }
    if (fromBlock === 0n) {
      break;
    }
    toBlock = fromBlock - 1n;
  }

  if (!matchingEvent) {
    throw new Error(`Aggregation event with sequence ${latestAggSeq} not found`);
  }

  let rootFromEvent: string;
  try {
    rootFromEvent = zeroPadValue(normalizeHex(matchingEvent.args.root), 32);
  } catch (error) {
    throw new Error(
      `Aggregation event root does not fit in 32 bytes: ${
        matchingEvent.args.root
      } (${error instanceof Error ? error.message : String(error)})`,
    );
  }
  if (rootFromEvent !== onchainRoot) {
    throw new Error(
      `Mismatch in aggregation root for seq ${latestAggSeq}: on-chain ${onchainRoot}, event ${rootFromEvent}`,
    );
  }

  const snapshot: string[] = (matchingEvent.args.transferRootsSnapshot ?? []).map((value: any, index: number) => {
    const normalized = normalizeHex(value);
    try {
      return zeroPadValue(normalized, 32);
    } catch (error) {
      throw new Error(
        `Aggregation snapshot value at index ${index} does not fit in 32 bytes: ${normalized} (${
          error instanceof Error ? error.message : String(error)
        })`,
      );
    }
  });
  const recomputedRoot = zeroPadValue(await aggregationRoot(snapshot), 32);
  if (recomputedRoot !== onchainRoot) {
    throw new Error(`Aggregation snapshot root mismatch: expected ${onchainRoot}, got ${recomputedRoot}`);
  }

  const transferTreeIndices: bigint[] = (matchingEvent.args.transferTreeIndicesSnapshot ?? []).map((value: any) =>
    BigInt(value),
  );

  const tokenInfos = await params.hubContract.getTokenInfos();
  const chainIds: bigint[] = tokenInfos.map((info: any) => BigInt(info.chainId));

  return {
    latestAggSeq,
    aggregationRoot: onchainRoot,
    snapshot,
    transferTreeIndices,
    chainIds,
  };
}

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

export function partitionEventsByEligibility(events: IndexedEvent[], treeRootIndex: bigint): EventsWithEligibility {
  const eligible: IndexedEvent[] = [];
  const ineligible: IndexedEvent[] = [];
  for (const event of events) {
    if (event.eventIndex < treeRootIndex) {
      eligible.push(event);
    } else {
      ineligible.push(event);
    }
  }
  return { eligible, ineligible };
}

export async function generateGlobalTeleportProofs(
  state: AggregationTreeState,
  chainId: bigint,
  events: readonly IndexedEvent[],
  localProofs: readonly HistoricalProof[],
): Promise<GlobalTeleportProofWithEvent[]> {
  if (events.length !== localProofs.length) {
    throw new Error(`Events length ${events.length} does not match proofs length ${localProofs.length}`);
  }
  const aggregationPosition = state.chainIds.findIndex((id) => id === chainId);
  if (aggregationPosition === -1) {
    throw new Error(`Chain id ${chainId} not found in aggregation state`);
  }
  const aggregationIndex = BigInt(aggregationPosition);
  const aggregationProof = await aggregationMerkleProof(state.snapshot, aggregationPosition);
  return localProofs.map((proof, idx) => {
    const event = events[idx];
    const leafIndex = BigInt(proof.leafIndex);
    const siblings = proof.siblings.map(normalizeHex);
    if (siblings.length !== TRANSFER_TREE_HEIGHT) {
      throw new Error(`Expected ${TRANSFER_TREE_HEIGHT} local proof siblings, received ${siblings.length}`);
    }
    const combinedSiblings = [...siblings, ...aggregationProof];
    if (combinedSiblings.length !== TRANSFER_TREE_HEIGHT + AGGREGATION_TREE_HEIGHT) {
      throw new Error('Combined global proof has unexpected length');
    }
    const globalLeafIndex =
      (aggregationIndex << BigInt(TRANSFER_TREE_HEIGHT)) + leafIndex;
    return {
      event,
      siblings: combinedSiblings,
      leafIndex: globalLeafIndex,
    };
  });
}
