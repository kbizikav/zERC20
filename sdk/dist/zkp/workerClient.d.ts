import type { NovaProverInput, NovaProverOutput, SingleTeleportArtifacts, SingleTeleportParams } from '../types.js';
type JobType = 'singleTeleport' | 'nova';
interface WorkerPayloads {
    singleTeleport: SingleTeleportParams;
    nova: NovaProverInput;
}
interface WorkerResults {
    singleTeleport: SingleTeleportArtifacts;
    nova: NovaProverOutput;
}
export declare function canUseZkpWorker(): boolean;
export declare function runZkpWorkerJob<T extends JobType>(type: T, payload: WorkerPayloads[T]): Promise<WorkerResults[T]>;
export declare function disableZkpWorker(error?: unknown): void;
export {};
//# sourceMappingURL=workerClient.d.ts.map