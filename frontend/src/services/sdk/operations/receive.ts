import { Contract, JsonRpcProvider } from 'ethers';

import { DEFAULT_INDEXER_FETCH_LIMIT } from '../core/constants.js';
import { HttpIndexerClient, HistoricalProof, IndexedEvent } from '../indexer/index.js';
import { HttpDeciderClient } from '../proofs/prover.js';
import {
  AggregationTreeState,
  EventsWithEligibility,
  fetchAggregationTreeState,
  generateGlobalTeleportProofs,
  getTreeIndexForChain,
  partitionEventsByEligibility,
  GlobalTeleportProof,
} from './teleport.js';
import { TokenEntry, findTokenByChain } from '../registry/tokens.js';
import { BurnArtifacts } from '../core/types.js';
import { createSingleWithdrawWasm, SingleWithdrawWasm } from '../wasm/index.js';
import { normalizeHex } from '../core/utils.js';
import { formatFieldElement, toFieldHex, toLeafIndexString } from './proofUtils.js';
import { runNovaProver, type NovaProverInput, type NovaProverOutput } from './novaProver.js';

export interface RedeemChainContext {
  chainId: bigint;
  token: TokenEntry;
  events: EventsWithEligibility;
  globalProofs: GlobalTeleportProof[];
  eligibleProofs: HistoricalProof[];
  totalEligibleValue: bigint;
  totalPendingValue: bigint;
  totalIndexedValue: bigint;
}

export interface RedeemContext {
  token: TokenEntry;
  aggregationState: AggregationTreeState;
  events: EventsWithEligibility;
  globalProofs: GlobalTeleportProof[];
  eligibleProofs: HistoricalProof[];
  totalEligibleValue: bigint;
  totalPendingValue: bigint;
  totalIndexedValue: bigint;
  totalTeleported: bigint;
  chains: RedeemChainContext[];
}

export interface RedeemContextParams {
  burn: BurnArtifacts;
  tokens: readonly TokenEntry[];
  indexer: HttpIndexerClient;
  verifierContract: Contract;
  hubContract: Contract;
  provider: JsonRpcProvider;
  indexerFetchLimit?: number;
  eventBlockSpan?: bigint;
}

export async function collectRedeemContext(params: RedeemContextParams): Promise<RedeemContext> {
  const primaryChainId = params.burn.generalRecipient.chainId;
  const primaryToken = findTokenByChain(params.tokens, primaryChainId);

  const aggregationState = await fetchAggregationTreeState({
    hubContract: params.hubContract,
    verifierContract: params.verifierContract,
    provider: params.provider,
    eventBlockSpan: params.eventBlockSpan,
  });

  const perChain: RedeemChainContext[] = [];

  for (const tokenEntry of params.tokens) {
    const events = await params.indexer.eventsByRecipient({
      chainId: tokenEntry.chainId,
      tokenAddress: tokenEntry.tokenAddress,
      to: params.burn.burnAddress,
      limit: params.indexerFetchLimit ?? DEFAULT_INDEXER_FETCH_LIMIT,
    });

    const treeRootIndex = getTreeIndexForChain(aggregationState, tokenEntry.chainId);
    const separated = partitionEventsByEligibility(events, treeRootIndex);
    const totalEligibleValue = separated.eligible.reduce<bigint>(
      (acc, event) => acc + event.value,
      0n,
    );
    const totalPendingValue = separated.ineligible.reduce<bigint>(
      (acc, event) => acc + event.value,
      0n,
    );
    const totalIndexedValue = totalEligibleValue + totalPendingValue;

    let globalProofs: GlobalTeleportProof[] = [];
    let eligibleProofs: HistoricalProof[] = [];

    if (separated.eligible.length > 0) {
      const leafIndices = separated.eligible.map((event) => event.eventIndex);
      const localProofs = await params.indexer.proveMany({
        chainId: tokenEntry.chainId,
        tokenAddress: tokenEntry.tokenAddress,
        targetIndex: treeRootIndex,
        leafIndices,
      });
      if (localProofs.length !== separated.eligible.length) {
        throw new Error(
          `Indexer returned ${localProofs.length} proofs but expected ${separated.eligible.length} for chain ${tokenEntry.chainId}`,
        );
      }

      globalProofs = await generateGlobalTeleportProofs(
        aggregationState,
        tokenEntry.chainId,
        separated.eligible,
        localProofs,
      );
      eligibleProofs = localProofs;
    }

    perChain.push({
      chainId: tokenEntry.chainId,
      token: tokenEntry,
      events: separated,
      globalProofs,
      eligibleProofs,
      totalEligibleValue,
      totalPendingValue,
      totalIndexedValue,
    });
  }

  const combinedEvents: EventsWithEligibility = {
    eligible: [],
    ineligible: [],
  };
  for (const chainContext of perChain) {
    combinedEvents.eligible.push(...chainContext.events.eligible);
    combinedEvents.ineligible.push(...chainContext.events.ineligible);
  }

  const combinedGlobalProofs = perChain.flatMap((chainContext) => chainContext.globalProofs);
  const combinedEligibleProofs = perChain.flatMap((chainContext) => chainContext.eligibleProofs);

  const totalEligibleValue = perChain.reduce<bigint>(
    (acc, chainContext) => acc + chainContext.totalEligibleValue,
    0n,
  );
  const totalPendingValue = perChain.reduce<bigint>(
    (acc, chainContext) => acc + chainContext.totalPendingValue,
    0n,
  );
  const totalIndexedValue = totalEligibleValue + totalPendingValue;

  const totalTeleported = BigInt(
    await params.verifierContract.totalTeleported(params.burn.generalRecipient.fr),
  );

  return {
    token: primaryToken,
    aggregationState,
    events: combinedEvents,
    globalProofs: combinedGlobalProofs,
    eligibleProofs: combinedEligibleProofs,
    totalEligibleValue,
    totalPendingValue,
    totalIndexedValue,
    totalTeleported,
    chains: perChain,
  };
}

export interface SingleTeleportArtifacts {
  proofCalldata: string;
  publicInputs: string[];
  treeDepth: number;
}

export interface SingleTeleportParams {
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

export async function generateSingleTeleportProof(params: SingleTeleportParams): Promise<SingleTeleportArtifacts> {
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
    // Surface detailed witness context for troubleshooting, but avoid leaking secrets when possible.
    // We redact the secret while keeping parity with CLI-style logs.
    const debugWitness = {
      ...witness,
      secret: '[redacted]',
    };
    console.error(
      '[zkERC20] generateSingleTeleportProof failed',
      debugWitness,
      error,
    );
    throw new Error(
      error instanceof Error
        ? `single teleport proof failed: ${error.message}`
        : `single teleport proof failed: ${String(error)}`,
    );
  }
}

export interface BatchTeleportArtifacts {
  deciderProof: Uint8Array;
  ivcProof: Uint8Array;
  finalState: string[];
  steps: number;
}

export interface BatchTeleportParams extends NovaProverInput {
  decider: HttpDeciderClient;
  onDeciderRequestStart?: () => void | Promise<void>;
  offloadToWorker?: boolean;
}

type PendingNovaJob = {
  resolve: (output: NovaProverOutput) => void;
  reject: (error: unknown) => void;
};

let novaWorker: Worker | null = null;
let nextNovaJobId = 0;
const pendingNovaJobs = new Map<number, PendingNovaJob>();

function canUseNovaWorker(params: BatchTeleportParams): boolean {
  if (params.offloadToWorker === false) {
    return false;
  }
  return typeof window !== 'undefined' && typeof window.Worker !== 'undefined';
}

function ensureNovaWorker(): Worker {
  if (!novaWorker) {
    const worker = new Worker(new URL('./novaProver.worker.ts', import.meta.url), { type: 'module' });
    worker.addEventListener('message', (event: MessageEvent<any>) => {
      const { id, type } = event.data ?? {};
      if (!pendingNovaJobs.has(id)) {
        return;
      }
      const pending = pendingNovaJobs.get(id);
      if (!pending) {
        return;
      }
      pendingNovaJobs.delete(id);
      if (type === 'result') {
        pending.resolve(event.data.result as NovaProverOutput);
      } else if (type === 'error') {
        const error = event.data.error;
        pending.reject(new Error(error?.message ?? 'Nova worker failure'));
      } else {
        pending.reject(new Error(`Nova worker returned unexpected message type ${String(type)}`));
      }
    });
    worker.addEventListener('error', (event) => {
      pendingNovaJobs.forEach((pending) => pending.reject(event.error ?? new Error('Nova worker error')));
      pendingNovaJobs.clear();
    });
    novaWorker = worker;
  }
  return novaWorker;
}

async function runNovaProofWithWorker(params: NovaProverInput): Promise<NovaProverOutput> {
  const worker = ensureNovaWorker();
  const jobId = nextNovaJobId++;
  const payload = { id: jobId, payload: params };
  const promise = new Promise<NovaProverOutput>((resolve, reject) => {
    pendingNovaJobs.set(jobId, { resolve, reject });
  });
  worker.postMessage(payload);
  return promise;
}

async function computeNovaProof(params: BatchTeleportParams): Promise<NovaProverOutput> {
  const workload: NovaProverInput = {
    wasmArtifacts: params.wasmArtifacts,
    aggregationState: params.aggregationState,
    recipientFr: params.recipientFr,
    secretHex: params.secretHex,
    proofs: params.proofs,
    events: params.events,
  };
  if (canUseNovaWorker(params)) {
    try {
      return await runNovaProofWithWorker(workload);
    } catch (error) {
      console.warn('[zkERC20] Falling back to main-thread Nova prover due to worker failure.', error);
    }
  }
  return runNovaProver(workload);
}

export async function generateBatchTeleportProof(params: BatchTeleportParams): Promise<BatchTeleportArtifacts> {
  const novaResult = await computeNovaProof(params);
  if (params.onDeciderRequestStart) {
    await params.onDeciderRequestStart();
  }
  const deciderProof = await params.decider.produceDeciderProof('withdraw_global', novaResult.ivcProof);

  return {
    deciderProof,
    ivcProof: novaResult.ivcProof,
    finalState: novaResult.finalState,
    steps: novaResult.steps,
  };
}
