import { computeNovaProof, computeSingleTeleportProof } from './runtime.js';
import { canUseZkpWorker, disableZkpWorker, runZkpWorkerJob } from './workerClient.js';
function shouldUseWorker(options) {
    if (options?.offloadToWorker === false) {
        return false;
    }
    return canUseZkpWorker();
}
export async function generateSingleTeleportProof(params, options) {
    if (shouldUseWorker(options)) {
        try {
            return await runZkpWorkerJob('singleTeleport', params);
        }
        catch (error) {
            console.warn('[zkERC20] Falling back to main-thread single proof due to worker failure.', error);
            disableZkpWorker(error);
        }
    }
    return computeSingleTeleportProof(params);
}
export async function runNovaProver(params, options) {
    if (shouldUseWorker(options)) {
        try {
            return await runZkpWorkerJob('nova', params);
        }
        catch (error) {
            console.warn('[zkERC20] Falling back to main-thread Nova prover due to worker failure.', error);
            disableZkpWorker(error);
        }
    }
    return computeNovaProof(params);
}
//# sourceMappingURL=index.js.map