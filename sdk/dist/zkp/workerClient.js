let worker = null;
let workerFailed = false;
let nextJobId = 0;
const pendingJobs = new Map();
function hasWorkerSupport() {
    return typeof Worker !== 'undefined';
}
function handleWorkerMessage(event) {
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
    }
    else if (message.type === 'error') {
        const error = new Error(message.error?.message ?? 'ZKP worker failure');
        error.stack = message.error?.stack;
        pending.reject(error);
    }
    else {
        pending.reject(new Error(`ZKP worker returned unexpected message type ${message.type}`));
    }
}
function handleWorkerError(event) {
    pendingJobs.forEach((pending) => pending.reject(event.error ?? new Error('ZKP worker error')));
    pendingJobs.clear();
    workerFailed = true;
    if (worker) {
        worker.terminate();
        worker = null;
    }
}
function ensureWorker() {
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
        instance.addEventListener('error', (event) => handleWorkerError(event));
        worker = instance;
    }
    return worker;
}
export function canUseZkpWorker() {
    return hasWorkerSupport() && !workerFailed;
}
export async function runZkpWorkerJob(type, payload) {
    const instance = ensureWorker();
    const jobId = nextJobId++;
    const message = { id: jobId, type, payload };
    const promise = new Promise((resolve, reject) => {
        pendingJobs.set(jobId, {
            resolve: (value) => resolve(value),
            reject,
        });
    });
    instance.postMessage(message);
    return promise;
}
export function disableZkpWorker(error) {
    workerFailed = true;
    if (worker) {
        worker.terminate();
        worker = null;
    }
    pendingJobs.forEach((pending) => pending.reject(error ?? new Error('ZKP worker disabled')));
    pendingJobs.clear();
}
//# sourceMappingURL=workerClient.js.map