import type {
  NovaProverInput,
  NovaProverOutput,
  SingleTeleportArtifacts,
  SingleTeleportParams,
} from '../types.js';
import { computeNovaProof, computeSingleTeleportProof } from './runtime.js';
import { canUseZkpWorker, disableZkpWorker, runZkpWorkerJob } from './workerClient.js';

export interface ZkpWorkerOptions {
  offloadToWorker?: boolean;
}

function shouldUseWorker(options?: ZkpWorkerOptions): boolean {
  if (options?.offloadToWorker === false) {
    return false;
  }
  return canUseZkpWorker();
}

export async function generateSingleTeleportProof(
  params: SingleTeleportParams,
  options?: ZkpWorkerOptions,
): Promise<SingleTeleportArtifacts> {
  if (shouldUseWorker(options)) {
    try {
      return await runZkpWorkerJob('singleTeleport', params);
    } catch (error) {
      console.warn('[zkERC20] Falling back to main-thread single proof due to worker failure.', error);
      disableZkpWorker(error);
    }
  }
  return computeSingleTeleportProof(params);
}

export async function runNovaProver(
  params: NovaProverInput,
  options?: ZkpWorkerOptions,
): Promise<NovaProverOutput> {
  if (shouldUseWorker(options)) {
    try {
      return await runZkpWorkerJob('nova', params);
    } catch (error) {
      console.warn('[zkERC20] Falling back to main-thread Nova prover due to worker failure.', error);
      disableZkpWorker(error);
    }
  }
  return computeNovaProof(params);
}
