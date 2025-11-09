import {
  DEFAULT_DECIDER_POLL_INTERVAL_MS,
  DEFAULT_DECIDER_TIMEOUT_MS,
} from '../constants.js';
import { encode as encodeBase64, decode as decodeBase64 } from 'base64-arraybuffer';

export type CircuitKind = 'root' | 'withdraw_local' | 'withdraw_global';

export interface DeciderClientOptions {
  pollIntervalMs?: number;
  timeoutMs?: number;
  fetchImpl?: typeof fetch;
}

interface JobStatusResponse {
  job_id: string;
  circuit: CircuitKind;
  status: 'queued' | 'processing' | 'completed' | 'failed';
  result?: string | null;
  error?: string | null;
}

function ensureFetch(): typeof fetch {
  if (typeof fetch === 'undefined') {
    throw new Error('fetch is not available in the current environment');
  }
  return (input: RequestInfo | URL, init?: RequestInit) => fetch(input, init);
}

function toBase64(data: Uint8Array): string {
  const contiguous =
    data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? data : data.slice();
  const copy = Uint8Array.from(contiguous);
  return encodeBase64(copy.buffer);
}

function fromBase64(payload: string): Uint8Array {
  return new Uint8Array(decodeBase64(payload) as ArrayBufferLike);
}

function toUint8Array(data: Uint8Array | string): Uint8Array {
  if (data instanceof Uint8Array) {
    return data;
  }
  const normalized = data.trim();
  if (normalized.startsWith('0x') || normalized.startsWith('0X')) {
    const hex = normalized.slice(2);
    if (hex.length === 0) {
      return new Uint8Array();
    }
    if (hex.length % 2 === 1) {
      throw new Error('hex input must have even length');
    }
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < hex.length; i += 2) {
      bytes[i / 2] = parseInt(hex.slice(i, i + 2), 16);
    }
    return bytes;
  }
  throw new Error('ivc proof must be Uint8Array or 0x-prefixed hex string');
}

export class HttpDeciderClient {
  private readonly baseUrl: string;
  private readonly pollInterval: number;
  private readonly timeout: number;
  private readonly fetchFn: typeof fetch;

  constructor(baseUrl: string, options: DeciderClientOptions = {}) {
    if (!baseUrl.endsWith('/')) {
      this.baseUrl = `${baseUrl}/`;
    } else {
      this.baseUrl = baseUrl;
    }
    this.pollInterval = options.pollIntervalMs ?? DEFAULT_DECIDER_POLL_INTERVAL_MS;
    this.timeout = options.timeoutMs ?? DEFAULT_DECIDER_TIMEOUT_MS;
    this.fetchFn = options.fetchImpl ?? ensureFetch();
  }

  private url(path: string): string {
    return new URL(path, this.baseUrl).toString();
  }

  private async performFetch(url: string, init?: RequestInit): Promise<Response> {
    try {
      return await this.fetchFn(url, init);
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      throw new Error(`decider request to ${url} failed: ${reason}`);
    }
  }

  async produceDeciderProof(circuit: CircuitKind, ivcProof: Uint8Array | string): Promise<Uint8Array> {
    const jobId = typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : `job-${Date.now()}`;
    const payload = {
      job_id: jobId,
      circuit,
      ivc_proof: toBase64(toUint8Array(ivcProof)),
    };

    const submitUrl = this.url('jobs');
    const submit = await this.performFetch(submitUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!submit.ok) {
      throw new Error(`decider job submission failed with status ${submit.status}`);
    }

    const statusUrl = this.url(`jobs/${jobId}`);
    const started = Date.now();
    while (Date.now() - started <= this.timeout) {
      const response = await this.performFetch(statusUrl);
      if (!response.ok) {
        throw new Error(`decider job status failed with status ${response.status}`);
      }
      const status: JobStatusResponse = await response.json();
      switch (status.status) {
        case 'queued':
        case 'processing':
          await new Promise((resolve) => setTimeout(resolve, this.pollInterval));
          continue;
        case 'failed':
          throw new Error(`decider job ${jobId} failed: ${status.error ?? 'unknown error'}`);
        case 'completed': {
          const result = status.result;
          if (!result) {
            throw new Error(`decider job ${jobId} completed without result`);
          }
          return fromBase64(result);
        }
        default:
          throw new Error(`decider job ${jobId} returned unexpected status '${status.status}'`);
      }
    }
    throw new Error(`decider job ${jobId} timed out after ${this.timeout} ms`);
  }
}
