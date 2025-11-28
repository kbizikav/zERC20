import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HttpDeciderClient } from '../../proofs/prover.js';

const ZERO_FR = `0x${'00'.repeat(32)}`;
const DUMMY_RECIPIENT = `0x${'01'.repeat(32)}`;

type CryptoLike = {
  getRandomValues<T extends ArrayBufferView | ArrayBuffer>(buffer: T): T;
};

type MutableCryptoGlobal = {
  crypto?: CryptoLike;
};

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '../../../../../..');
const wasmPath = path.join(repoRoot, 'wasm', 'pkg', 'zkerc20_wasm_bg.wasm');

const previousOverride = (globalThis as { __ZKERC20_WASM_PATH__?: string }).__ZKERC20_WASM_PATH__;
(globalThis as { __ZKERC20_WASM_PATH__?: string }).__ZKERC20_WASM_PATH__ = pathToFileURL(wasmPath).toString();

const { generateBatchTeleportProof } = await import('../receive.js');

function loadArtifact(name: string): Uint8Array {
  const fullPath = path.join(repoRoot, 'nova_artifacts', name);
  const buffer = readFileSync(fullPath);
  return new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
}

const originalCrypto = (globalThis as unknown as MutableCryptoGlobal).crypto;
const originalFetch = globalThis.fetch;
let installedCrypto = false;
let previousGetRandomValues: CryptoLike['getRandomValues'] | undefined;

function deterministicRandomValues<T extends ArrayBufferView | ArrayBuffer>(buffer: T): T {
  if (buffer instanceof Uint32Array && buffer.length > 0) {
    buffer[0] = 2;
  }
  return buffer;
}

beforeAll(() => {
  (globalThis as { fetch?: typeof fetch }).fetch = (async (input: any, init?: any) => {
    const resolveUrl = (candidate: any): string | undefined => {
      if (typeof candidate === 'string') {
        return candidate;
      }
      if (candidate && typeof candidate === 'object') {
        if (candidate instanceof URL) {
          return candidate.toString();
        }
        if ('url' in candidate && typeof candidate.url === 'string') {
          return candidate.url;
        }
      }
      return undefined;
    };

    const target = resolveUrl(input);
    if (target && target.startsWith('file://')) {
      const wasmFile = fileURLToPath(new URL(target));
      const bytes = readFileSync(wasmFile);
      return new Response(bytes, {
        status: 200,
        headers: { 'Content-Type': 'application/wasm' },
      });
    }

    if (originalFetch) {
      return originalFetch.call(globalThis, input, init);
    }
    throw new Error('fetch is not available in this environment');
  }) as typeof fetch;

  const globalRef = globalThis as unknown as MutableCryptoGlobal;
  if (!globalRef.crypto) {
    globalRef.crypto = {
      getRandomValues: deterministicRandomValues,
    };
    installedCrypto = true;
  } else {
    previousGetRandomValues = globalRef.crypto.getRandomValues.bind(globalRef.crypto);
    globalRef.crypto.getRandomValues = deterministicRandomValues;
  }
});

afterAll(() => {
  const globalRef = globalThis as unknown as MutableCryptoGlobal;
  if (installedCrypto) {
    if (originalCrypto) {
      globalRef.crypto = originalCrypto;
    } else {
      delete globalRef.crypto;
    }
  } else if (globalRef.crypto && previousGetRandomValues) {
    globalRef.crypto.getRandomValues = previousGetRandomValues;
  }
  installedCrypto = false;
  previousGetRandomValues = undefined;
  if (originalFetch) {
    (globalThis as { fetch?: typeof fetch }).fetch = originalFetch;
  } else {
    delete (globalThis as { fetch?: typeof fetch }).fetch;
  }
  if (previousOverride) {
    (globalThis as { __ZKERC20_WASM_PATH__?: string }).__ZKERC20_WASM_PATH__ = previousOverride;
  } else {
    delete (globalThis as { __ZKERC20_WASM_PATH__?: string }).__ZKERC20_WASM_PATH__;
  }
});

describe('generateBatchTeleportProof (dummy steps)', () => {
  let produceDeciderProofSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    produceDeciderProofSpy = vi.fn(async (circuit: string, proofBytes: Uint8Array) => {
      expect(circuit).toBe('withdraw_global');
      expect(proofBytes.byteLength).toBeGreaterThan(0);
      return Uint8Array.from([0xde, 0xad, 0xbe, 0xef]);
    });
  });

  afterEach(() => {
    produceDeciderProofSpy.mockReset();
  });

  it('produces a real withdraw nova proof using two dummy steps', async () => {
    const expectedDummySteps = 2;

    const deciderMock = {
      produceDeciderProof: produceDeciderProofSpy,
    } as unknown as HttpDeciderClient;

    const result = await generateBatchTeleportProof({
      wasmArtifacts: {
        localPp: loadArtifact('withdraw_local_nova_pp.bin'),
        localVp: loadArtifact('withdraw_local_nova_vp.bin'),
        globalPp: loadArtifact('withdraw_global_nova_pp.bin'),
        globalVp: loadArtifact('withdraw_global_nova_vp.bin'),
      },
      aggregationState: {
        latestAggSeq: 1n,
        aggregationRoot: ZERO_FR,
        snapshot: [],
        transferTreeIndices: [],
        chainIds: [],
      },
      recipientFr: DUMMY_RECIPIENT,
      secretHex: '0x0',
      proofs: [],
      events: [],
      decider: deciderMock,
    });

    expect(produceDeciderProofSpy).toHaveBeenCalledTimes(1);
    const [circuitArg, proofArg] = produceDeciderProofSpy.mock.calls[0] as [string, Uint8Array];
    expect(circuitArg).toBe('withdraw_global');
    expect(proofArg.byteLength).toBeGreaterThan(0);

    expect(result.steps).toBe(expectedDummySteps);
    expect(result.ivcProof.byteLength).toBeGreaterThan(0);
    expect(result.deciderProof).toEqual(Uint8Array.from([0xde, 0xad, 0xbe, 0xef]));
    expect(result.finalState).toHaveLength(4);
    expect(result.finalState[0]).toBe(ZERO_FR);
    expect(result.finalState[1]).toBe(DUMMY_RECIPIENT);

  }, 20000);
});
