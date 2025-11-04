import { runNovaProver } from './novaProver.js';
import type { NovaProverInput, NovaProverOutput } from './novaProver.js';

interface WorkerRequest {
  id: number;
  payload: NovaProverInput;
}

interface ResultMessage {
  id: number;
  type: 'result';
  result: NovaProverOutput;
}

interface ErrorMessage {
  id: number;
  type: 'error';
  error: { message: string; stack?: string };
}

const ctx: any = self as any;

ctx.onmessage = async (event: MessageEvent<WorkerRequest>) => {
  const { id, payload } = event.data;
  try {
    const result = await runNovaProver(payload);
    const message: ResultMessage = { id, type: 'result', result };
    ctx.postMessage(message);
  } catch (error) {
    const normalized: ErrorMessage = {
      id,
      type: 'error',
      error: {
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
      },
    };
    ctx.postMessage(normalized);
  }
};

export {};
