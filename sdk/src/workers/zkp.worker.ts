import { computeNovaProof, computeSingleTeleportProof } from '../zkp/runtime.js';
import type {
  NovaProverInput,
  NovaProverOutput,
  SingleTeleportArtifacts,
  SingleTeleportParams,
} from '../types.js';

type WorkerJob =
  | { id: number; type: 'singleTeleport'; payload: SingleTeleportParams }
  | { id: number; type: 'nova'; payload: NovaProverInput };

type WorkerResponse =
  | { id: number; type: 'result'; result: SingleTeleportArtifacts | NovaProverOutput }
  | { id: number; type: 'error'; error: { message: string; stack?: string } };

const ctx: any = self as any;

ctx.addEventListener('message', async (event: MessageEvent<WorkerJob>) => {
  const message = event.data;
  if (!message) {
    return;
  }
  try {
    let payload: SingleTeleportArtifacts | NovaProverOutput;
    if (message.type === 'singleTeleport') {
      payload = await computeSingleTeleportProof(message.payload);
    } else if (message.type === 'nova') {
      payload = await computeNovaProof(message.payload);
    } else {
      throw new Error(`Unknown ZKP worker job type ${(message as WorkerJob).type}`);
    }
    const response: WorkerResponse = { id: message.id, type: 'result', result: payload };
    ctx.postMessage(response);
  } catch (error) {
    const response: WorkerResponse = {
      id: message.id,
      type: 'error',
      error: {
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
      },
    };
    ctx.postMessage(response);
  }
});
