import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { WasmRuntime } from "../../wasm/index.js";
import { ProofService } from "../index.js";

const ZERO_FR = `0x${"00".repeat(32)}`;
const DUMMY_RECIPIENT = `0x${"01".repeat(32)}`;

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "../../../../");
const wasmPath = path.join(
  repoRoot,
  "sdk",
  "src",
  "assets",
  "wasm",
  "zkerc20_wasm_bg.wasm"
);

const originalFetch = globalThis.fetch;
let proofs: ProofService;

beforeAll(() => {
  (globalThis as { fetch?: typeof fetch }).fetch = (async (
    input: any,
    init?: any
  ) => {
    const resolveUrl = (candidate: any): string | undefined => {
      if (typeof candidate === "string") {
        return candidate;
      }
      if (candidate && typeof candidate === "object") {
        if (candidate instanceof URL) {
          return candidate.toString();
        }
        if ("url" in candidate && typeof candidate.url === "string") {
          return candidate.url;
        }
      }
      return undefined;
    };

    const target = resolveUrl(input);
    if (target && target.startsWith("file://")) {
      const wasmFile = fileURLToPath(new URL(target));
      const bytes = readFileSync(wasmFile);
      return new Response(bytes, {
        status: 200,
        headers: { "Content-Type": "application/wasm" },
      });
    }

    if (originalFetch) {
      return originalFetch.call(globalThis, input, init);
    }
    throw new Error("fetch is not available in this environment");
  }) as typeof fetch;

  const wasm = new WasmRuntime({
    url: pathToFileURL(wasmPath).toString(),
  });
  proofs = new ProofService(wasm, { defaultToWorker: false });
});

afterAll(() => {
  if (originalFetch) {
    (globalThis as { fetch?: typeof fetch }).fetch = originalFetch;
  } else {
    delete (globalThis as { fetch?: typeof fetch }).fetch;
  }
  proofs = undefined as unknown as ProofService;
});

describe("runNovaProver (dummy steps)", () => {
  it("produces a withdraw nova proof using deterministic dummy steps", async () => {
    const result = await proofs.runNovaProver({
      aggregationState: {
        latestAggSeq: 1n,
        aggregationRoot: ZERO_FR,
        snapshot: [],
        transferTreeIndices: [],
        chainIds: [],
      },
      recipientFr: DUMMY_RECIPIENT,
      secretHex: "0x0",
      proofs: [],
      events: [],
    });

    expect(result.steps).toBeGreaterThanOrEqual(1);
    expect(result.steps).toBeLessThanOrEqual(3);
    expect(result.ivcProof.byteLength).toBeGreaterThan(0);
    expect(result.finalState).toHaveLength(4);
    expect(result.finalState[0]).toBe(ZERO_FR);
    expect(result.finalState[1]).toBe(DUMMY_RECIPIENT);
  }, 20_000);
});
