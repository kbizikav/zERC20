import type {
  NovaProverInput,
  NovaProverOutput,
  SingleTeleportArtifacts,
  SingleTeleportParams,
} from '../types.js';

type JobType = 'singleTeleport' | 'nova';

interface WorkerPayloads {
  singleTeleport: SingleTeleportParams;
  nova: NovaProverInput;
}

interface WorkerResults {
  singleTeleport: SingleTeleportArtifacts;
  nova: NovaProverOutput;
}

interface WorkerRequest<T extends JobType> {
  id: number;
  type: T;
  payload: WorkerPayloads[T];
}

interface WorkerResultMessage<T extends JobType> {
  id: number;
  type: 'result';
  result: WorkerResults[T];
}

interface WorkerErrorMessage {
  id: number;
  type: 'error';
  error: { message: string; stack?: string };
}

type PendingJob = {
  resolve: (value: WorkerResults[JobType]) => void;
  reject: (reason: unknown) => void;
};

let worker: Worker | null = null;
let workerFailed = false;
let nextJobId = 0;
const pendingJobs = new Map<number, PendingJob>();

function hasWorkerSupport(): boolean {
  return typeof Worker !== 'undefined';
}

function handleWorkerMessage(event: MessageEvent<WorkerResultMessage<JobType> | WorkerErrorMessage>): void {
  const message = event.data;
  if (!message || typeof message !== 'object') {
    return;
  }
  const pending = pendingJobs.get(message.id);
  if (!pending) {
    return;
  }
  pendingJobs.delete(message.id);
  if (message.type === 'result') {
    pending.resolve(message.result);
  } else if (message.type === 'error') {
    const error = new Error(message.error?.message ?? 'ZKP worker failure');
    error.stack = message.error?.stack;
    pending.reject(error);
  } else {
    pending.reject(new Error(`ZKP worker returned unexpected message type ${(message as any).type}`));
  }
}

function handleWorkerError(event: ErrorEvent): void {
  pendingJobs.forEach((pending) => pending.reject(event.error ?? new Error('ZKP worker error')));
  pendingJobs.clear();
  workerFailed = true;
  if (worker) {
    worker.terminate();
    worker = null;
  }
}

function ensureWorker(): Worker {
  if (workerFailed) {
    throw new Error('ZKP worker previously failed and has been disabled');
  }
  if (!hasWorkerSupport()) {
    throw new Error('Web Worker API is not available in this environment');
  }
  if (!worker) {
    const workerUrl = new URL('../workers/zkp.worker.js', import.meta.url);
    const instance = new Worker(workerUrl, { type: 'module' });
    instance.addEventListener('message', handleWorkerMessage);
    instance.addEventListener('error', (event) => handleWorkerError(event as ErrorEvent));
    worker = instance;
  }
  return worker;
}

export function canUseZkpWorker(): boolean {
  return hasWorkerSupport() && !workerFailed;
}

export async function runZkpWorkerJob<T extends JobType>(
  type: T,
  payload: WorkerPayloads[T],
): Promise<WorkerResults[T]> {
  const instance = ensureWorker();
  const jobId = nextJobId++;
  const message: WorkerRequest<T> = { id: jobId, type, payload };
  const promise = new Promise<WorkerResults[T]>((resolve, reject) => {
    pendingJobs.set(jobId, {
      resolve: (value) => resolve(value as WorkerResults[T]),
      reject,
    });
  });
  instance.postMessage(message);
  return promise;
}

export function disableZkpWorker(error?: unknown): void {
  workerFailed = true;
  if (worker) {
    worker.terminate();
    worker = null;
  }
  pendingJobs.forEach((pending) =>
    pending.reject(error ?? new Error('ZKP worker disabled')),
  );
  pendingJobs.clear();
}
