import { DEFAULT_INDEXER_FETCH_LIMIT } from '../constants.js';
import { HttpDeciderClient } from '../decider/prover.js';
import { getTreeIndexForChain } from './teleport.js';
import { HubEntry, TokenEntry, findTokenByChain } from '../registry/tokens.js';
import {
  AggregationTreeState,
  EventsWithEligibility,
  GlobalTeleportProofWithEvent,
  BurnArtifacts,
  LocalTeleportProof,
  SingleTeleportArtifacts,
  SingleTeleportParams,
  NovaProverInput,
  NovaProverOutput,
} from '../types.js';
import {
  createSingleWithdrawWasm,
  SingleWithdrawWasm,
  fetchAggregationTreeState,
  fetchTransferEvents,
  separateEventsByEligibility,
  fetchLocalTeleportMerkleProofs as fetchLocalTeleportProofs,
  generateGlobalTeleportMerkleProofs as generateGlobalTeleportProofs,
  getDefaultWasmRuntime,
} from '../wasm/index.js';
import { loadSingleTeleportArtifacts } from '../wasm/artifacts.js';
import { normalizeHex } from '../utils/hex.js';
import { formatFieldElement, toFieldHex, toLeafIndexString } from '../zkp/proofUtils.js';
import { ProofService } from '../zkp/proofService.js';

const proofService = new ProofService(getDefaultWasmRuntime());

export interface RedeemChainContext {
  chainId: bigint;
  token: TokenEntry;
  events: EventsWithEligibility;
  globalProofs: GlobalTeleportProofWithEvent[];
  eligibleProofs: LocalTeleportProof[];
  totalEligibleValue: bigint;
  totalPendingValue: bigint;
  totalIndexedValue: bigint;
}

export interface RedeemContext {
  token: TokenEntry;
  aggregationState: AggregationTreeState;
  events: EventsWithEligibility;
  globalProofs: GlobalTeleportProofWithEvent[];
  eligibleProofs: LocalTeleportProof[];
  totalEligibleValue: bigint;
  totalPendingValue: bigint;
  totalIndexedValue: bigint;
  totalTeleported: bigint;
  chains: RedeemChainContext[];
}

type ViemVerifierContract = {
  read: {
    totalTeleported: (args: readonly [string]) => Promise<bigint | number | string>;
  };
};

type LegacyVerifierContract = {
  totalTeleported: (recipientFr: string) => Promise<bigint | number | string>;
};

type VerifierContractLike = ViemVerifierContract | LegacyVerifierContract;

export interface RedeemContextParams {
  burn: BurnArtifacts;
  tokens: readonly TokenEntry[];
  hub: HubEntry;
  verifierContract: VerifierContractLike;
  indexerUrl: string;
  indexerFetchLimit?: number;
  eventBlockSpan?: bigint | number;
}

export async function collectRedeemContext(params: RedeemContextParams): Promise<RedeemContext> {
  const primaryChainId = params.burn.generalRecipient.chainId;
  const primaryToken = findTokenByChain(params.tokens, primaryChainId);

  const aggregationState = await fetchAggregationTreeState({
    hub: params.hub,
    token: primaryToken,
    eventBlockSpan: params.eventBlockSpan,
  });

  const chainEvents = await fetchTransferEvents({
    indexerUrl: params.indexerUrl,
    tokens: params.tokens,
    burnAddresses: [params.burn.burnAddress],
    indexerFetchLimit: params.indexerFetchLimit ?? DEFAULT_INDEXER_FETCH_LIMIT,
  });

  const separatedChains = await separateEventsByEligibility({
    aggregationState,
    events: chainEvents,
  });

  const eventsByChain = new Map<bigint, EventsWithEligibility>();
  for (const entry of separatedChains) {
    eventsByChain.set(entry.chainId, entry.events);
  }

  const perChain: RedeemChainContext[] = [];
  const combinedEvents: EventsWithEligibility = { eligible: [], ineligible: [] };
  const combinedGlobalProofs: GlobalTeleportProofWithEvent[] = [];
  const combinedEligibleProofs: LocalTeleportProof[] = [];

  for (const tokenEntry of params.tokens) {
    const chainEventsForToken =
      eventsByChain.get(tokenEntry.chainId) ?? { eligible: [], ineligible: [] };

    combinedEvents.eligible.push(...chainEventsForToken.eligible);
    combinedEvents.ineligible.push(...chainEventsForToken.ineligible);

    const totalEligibleValue = chainEventsForToken.eligible.reduce<bigint>(
      (acc, event) => acc + event.value,
      0n,
    );
    const totalPendingValue = chainEventsForToken.ineligible.reduce<bigint>(
      (acc, event) => acc + event.value,
      0n,
    );
    const totalIndexedValue = totalEligibleValue + totalPendingValue;

    let globalProofs: GlobalTeleportProofWithEvent[] = [];
    let eligibleProofs: LocalTeleportProof[] = [];

    if (chainEventsForToken.eligible.length > 0) {
      const treeRootIndex = getTreeIndexForChain(aggregationState, tokenEntry.chainId);
      eligibleProofs = await fetchLocalTeleportProofs({
        indexerUrl: params.indexerUrl,
        token: tokenEntry,
        treeIndex: treeRootIndex,
        events: chainEventsForToken.eligible,
      });
      globalProofs = await generateGlobalTeleportProofs({
        aggregationState,
        chains: [
          {
            chainId: tokenEntry.chainId,
            proofs: eligibleProofs,
          },
        ],
      });
    }

    perChain.push({
      chainId: tokenEntry.chainId,
      token: tokenEntry,
      events: chainEventsForToken,
      globalProofs,
      eligibleProofs,
      totalEligibleValue,
      totalPendingValue,
      totalIndexedValue,
    });

    combinedGlobalProofs.push(...globalProofs);
    combinedEligibleProofs.push(...eligibleProofs);
  }

  const totalEligibleValue = perChain.reduce<bigint>(
    (acc, chainContext) => acc + chainContext.totalEligibleValue,
    0n,
  );
  const totalPendingValue = perChain.reduce<bigint>(
    (acc, chainContext) => acc + chainContext.totalPendingValue,
    0n,
  );
  const totalIndexedValue = totalEligibleValue + totalPendingValue;

  const recipientFr = normalizeHex(params.burn.generalRecipient.fr);
  const totalTeleported = await readTotalTeleported(params.verifierContract, recipientFr);

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

function isViemVerifierContract(contract: VerifierContractLike): contract is ViemVerifierContract {
  return typeof (contract as ViemVerifierContract).read?.totalTeleported === 'function';
}

async function readTotalTeleported(
  contract: VerifierContractLike,
  recipientFr: string,
): Promise<bigint> {
  if (isViemVerifierContract(contract)) {
    const value = await contract.read.totalTeleported([recipientFr]);
    return BigInt(value as bigint | number | string);
  }
  if (typeof (contract as LegacyVerifierContract).totalTeleported === 'function') {
    const value = await (contract as LegacyVerifierContract).totalTeleported(recipientFr);
    return BigInt(value as bigint | number | string);
  }
  throw new Error('verifier contract must expose totalTeleported');
}

export async function generateSingleTeleportProof(params: SingleTeleportParams): Promise<SingleTeleportArtifacts> {
  const wasmArtifacts = await loadSingleTeleportArtifacts();
  const wasm: SingleWithdrawWasm = await createSingleWithdrawWasm(
    wasmArtifacts.localPk,
    wasmArtifacts.localVk,
    wasmArtifacts.globalPk,
    wasmArtifacts.globalVk,
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

async function computeNovaProof(params: BatchTeleportParams): Promise<NovaProverOutput> {
  const workload: NovaProverInput = {
    aggregationState: params.aggregationState,
    recipientFr: params.recipientFr,
    secretHex: params.secretHex,
    proofs: params.proofs,
    events: params.events,
  };
  return proofService.runNovaProver(workload, {
    offloadToWorker: params.offloadToWorker,
  });
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
