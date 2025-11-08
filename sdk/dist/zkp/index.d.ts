import type { NovaProverInput, NovaProverOutput, SingleTeleportArtifacts, SingleTeleportParams } from '../types.js';
export interface ZkpWorkerOptions {
    offloadToWorker?: boolean;
}
export declare function generateSingleTeleportProof(params: SingleTeleportParams, options?: ZkpWorkerOptions): Promise<SingleTeleportArtifacts>;
export declare function runNovaProver(params: NovaProverInput, options?: ZkpWorkerOptions): Promise<NovaProverOutput>;
//# sourceMappingURL=index.d.ts.map