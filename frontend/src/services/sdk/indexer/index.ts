import { normalizeHex } from '../core/utils.js';

export interface IndexedEvent {
  eventIndex: bigint;
  from: string;
  to: string;
  value: bigint;
  ethBlockNumber: bigint;
}

export interface HistoricalProof {
  targetIndex: bigint;
  leafIndex: bigint;
  root: string;
  hashChain: string;
  siblings: string[];
}

export interface EventsQueryParams {
  chainId: bigint;
  tokenAddress: string;
  to: string;
  limit?: number;
}

export interface ProveManyParams {
  chainId: bigint;
  tokenAddress: string;
  targetIndex: bigint;
  leafIndices: bigint[];
}

function toBigInt(value: unknown, label: string): bigint {
  if (typeof value === 'bigint') {
    return value;
  }
  if (typeof value === 'number') {
    return BigInt(value);
  }
  if (typeof value === 'string') {
    const normalized = value.trim();
    if (normalized.startsWith('0x') || normalized.startsWith('0X')) {
      return BigInt(normalized);
    }
    if (/^\d+$/.test(normalized)) {
      return BigInt(normalized);
    }
  }
  throw new Error(`${label} must be a bigint-compatible value`);
}

function toSafeNumber(value: bigint | number, label: string): number {
  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value)) {
      throw new Error(`${label} must be a safe integer, received ${value}`);
    }
    return value;
  }
  const max = BigInt(Number.MAX_SAFE_INTEGER);
  const min = BigInt(Number.MIN_SAFE_INTEGER);
  if (value > max || value < min) {
    throw new Error(`${label} ${value.toString()} exceeds the safe integer range`);
  }
  return Number(value);
}

function asIndexedEvent(value: any): IndexedEvent {
  return {
    eventIndex: toBigInt(value.event_index ?? value.eventIndex, 'eventIndex'),
    from: normalizeHex(value.from),
    to: normalizeHex(value.to),
    value: toBigInt(value.value, 'value'),
    ethBlockNumber: toBigInt(value.eth_block_number ?? value.ethBlockNumber, 'ethBlockNumber'),
  };
}

function asHistoricalProof(value: any): HistoricalProof {
  return {
    targetIndex: toBigInt(value.target_index ?? value.targetIndex, 'targetIndex'),
    leafIndex: toBigInt(value.leaf_index ?? value.leafIndex, 'leafIndex'),
    root: normalizeHex(value.root),
    hashChain: normalizeHex(value.hash_chain ?? value.hashChain),
    siblings: Array.isArray(value.siblings)
      ? value.siblings.map((s: any) => normalizeHex(String(s)))
      : [],
  };
}

function ensureFetch(): typeof fetch {
  if (typeof fetch === 'undefined') {
    throw new Error('fetch is not available in the current environment');
  }
  return (input: RequestInfo | URL, init?: RequestInit) => fetch(input, init);
}

export class HttpIndexerClient {
  private readonly baseUrl: string;
  private readonly fetchFn: typeof fetch;

  constructor(baseUrl: string, fetchImpl?: typeof fetch) {
    if (!baseUrl.endsWith('/')) {
      this.baseUrl = `${baseUrl}/`;
    } else {
      this.baseUrl = baseUrl;
    }
    this.fetchFn = fetchImpl ?? ensureFetch();
  }

  private url(path: string): string {
    return new URL(path, this.baseUrl).toString();
  }

  private async performFetch(url: string, init?: RequestInit): Promise<Response> {
    try {
      return await this.fetchFn(url, init);
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      throw new Error(`indexer request to ${url} failed: ${reason}`);
    }
  }

  async eventsByRecipient(params: EventsQueryParams): Promise<IndexedEvent[]> {
    const query = new URLSearchParams({
      chain_id: params.chainId.toString(),
      token_address: normalizeHex(params.tokenAddress),
      to: normalizeHex(params.to),
    });
    if (params.limit !== undefined) {
      query.set('limit', params.limit.toString());
    }
    const requestUrl = `${this.url('events')}?${query.toString()}`;
    const response = await this.performFetch(requestUrl);
    if (!response.ok) {
      const detail = await safeResponseText(response);
      throw new Error(
        `indexer events request failed with status ${response.status}${detail}`,
      );
    }
    const body = await response.json();
    if (!Array.isArray(body)) {
      throw new Error('indexer events response must be an array');
    }
    return body.map(asIndexedEvent);
  }

  async proveMany(params: ProveManyParams): Promise<HistoricalProof[]> {
    const payload = {
      chain_id: toSafeNumber(params.chainId, 'chainId'),
      token_address: normalizeHex(params.tokenAddress),
      target_index: toSafeNumber(params.targetIndex, 'targetIndex'),
      leaf_indices: params.leafIndices.map((idx, position) =>
        toSafeNumber(idx, `leafIndices[${position}]`),
      ),
    };
    const requestUrl = this.url('proofs');
    const response = await this.performFetch(requestUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!response.ok) {
      const detail = await safeResponseText(response);
      throw new Error(
        `indexer proofs request failed with status ${response.status}${detail}`,
      );
    }
    const body = await response.json();
    if (!Array.isArray(body)) {
      throw new Error('indexer proofs response must be an array');
    }
    return body.map(asHistoricalProof);
  }

  async treeIndexByRoot(
    chainId: bigint,
    tokenAddress: string,
    transferRoot: bigint,
  ): Promise<bigint> {
    const query = new URLSearchParams({
      chain_id: chainId.toString(),
      token_address: normalizeHex(tokenAddress),
      transfer_root: normalizeHex(`0x${transferRoot.toString(16)}`),
    });
    const requestUrl = `${this.url('tree-index')}?${query.toString()}`;
    const response = await this.performFetch(requestUrl);
    if (!response.ok) {
      const detail = await safeResponseText(response);
      throw new Error(
        `indexer tree-index request failed with status ${response.status}${detail}`,
      );
    }
    const body = await response.json();
    if (typeof body !== 'object' || body === null || body.tree_index === undefined) {
      throw new Error('indexer tree-index response is malformed');
    }
    return toBigInt(body.tree_index ?? body.treeIndex, 'treeIndex');
  }
}

async function safeResponseText(response: Response): Promise<string> {
  try {
    const text = await response.text();
    const trimmed = text.trim();
    return trimmed.length > 0 ? `: ${trimmed}` : '';
  } catch {
    return '';
  }
}
