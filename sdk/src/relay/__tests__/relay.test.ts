import { afterEach, describe, expect, it, vi } from "vitest";
import type { Hex } from "viem";

import {
  estimateRelayFee,
  fetchSwapQuote,
  submitRelaySwap,
  submitRelayTeleport,
} from "../relay.js";

function bytes32Hex(value: number): Hex {
  return `0x${value.toString(16).padStart(64, "0")}`;
}

describe("relay helpers", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("submits a teleport relay request", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ txHash: "0xabc123" }),
    });
    vi.stubGlobal("fetch", mockFetch);

    const result = await submitRelayTeleport("http://localhost:3000", {
      isSingle: false,
      isGlobal: true,
      rootHint: 9n,
      chainId: 1n,
      recipient: bytes32Hex(0x11),
      tweak: bytes32Hex(0x22),
      proof: "0x1234" as Hex,
      relayerFee: 100n,
      maxFee: 100n,
      deadline: 9999999999n,
      signature: "0xabcd" as Hex,
    });

    expect(result.txHash).toBe("0xabc123");
    expect(mockFetch).toHaveBeenCalledWith(
      "http://localhost:3000/relay/teleport",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("throws on relay node error", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 500,
      text: () => Promise.resolve("internal error"),
    });
    vi.stubGlobal("fetch", mockFetch);

    await expect(
      submitRelayTeleport("http://localhost:3000", {
        isSingle: false,
        isGlobal: false,
        rootHint: 1n,
        chainId: 1n,
        recipient: bytes32Hex(0),
        tweak: bytes32Hex(0),
        proof: "0x" as Hex,
        relayerFee: 0n,
        maxFee: 0n,
        deadline: 0n,
        signature: "0x" as Hex,
      }),
    ).rejects.toThrow(/500/);
  });

  it("estimates relay fee from node", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ relayerFee: "12345" }),
    });
    vi.stubGlobal("fetch", mockFetch);

    const result = await estimateRelayFee("http://localhost:3000", 1);
    expect(result.relayerFee).toBe(12345n);
  });

  it("fetches a swap quote", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          nativeAmount: "1000000000000000",
          feeBps: 50,
          relayerFee: "21000000000000",
        }),
    });
    vi.stubGlobal("fetch", mockFetch);

    const result = await fetchSwapQuote(
      "http://localhost:3000",
      1,
      50000000000000000n,
    );
    expect(result.nativeAmount).toBe(1000000000000000n);
    expect(result.feeBps).toBe(50);
    expect(result.relayerFee).toBe(21000000000000n);
    expect(mockFetch).toHaveBeenCalledWith(
      expect.stringContaining("/relay/swap-quote?chain_id=1&amount="),
    );
  });

  it("submits a swap request", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          txHash: "0xaaa",
        }),
    });
    vi.stubGlobal("fetch", mockFetch);

    const result = await submitRelaySwap("http://localhost:3000", {
      chainId: 1,
      tokenAmount: 50000000000000000n,
      minNativeAmount: 900000000000000n,
      recipient: bytes32Hex(0x11) as `0x${string}`,
      owner: bytes32Hex(0x22) as `0x${string}`,
      permitDeadline: 9999999999n,
      permitV: 28,
      permitR: bytes32Hex(0x33),
      permitS: bytes32Hex(0x44),
    });

    expect(result.txHash).toBe("0xaaa");
    expect(mockFetch).toHaveBeenCalledWith(
      "http://localhost:3000/relay/swap",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("throws on swap quote error", async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 503,
      text: () => Promise.resolve("swap is not enabled"),
    });
    vi.stubGlobal("fetch", mockFetch);

    await expect(
      fetchSwapQuote("http://localhost:3000", 1, 100n),
    ).rejects.toThrow(/503/);
  });
});
