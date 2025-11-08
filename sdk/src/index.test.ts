import { describe, expect, it } from "vitest";
import { normalizeHex, toBigInt } from "./utils/hex.js";

describe("normalizeHex", () => {
  it("adds a prefix and lowercases the payload", () => {
    expect(normalizeHex("ABC123")).toBe("0xabc123");
  });

  it("handles uint8 inputs", () => {
    expect(normalizeHex(new Uint8Array([10, 255]))).toBe("0x0aff");
  });
});

describe("toBigInt", () => {
  it("parses numeric-like inputs", () => {
    expect(toBigInt("42")).toBe(42n);
    expect(toBigInt(7)).toBe(7n);
  });
});
