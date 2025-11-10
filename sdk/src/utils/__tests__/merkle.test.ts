import { describe, expect, it } from 'vitest';

import type { GlobalTeleportProof, IndexedEvent } from '../../types.js';
import {
  computeLeafHash,
  computeMerkleRootFromSiblings,
  verifyGlobalTeleportProofs,
} from '../merkle.js';

function asHex(value: bigint): string {
  return `0x${value.toString(16).padStart(64, '0')}`;
}

function makeEvent(overrides: Partial<IndexedEvent> = {}): IndexedEvent {
  return {
    eventIndex: overrides.eventIndex ?? 3n,
    from: overrides.from ?? '0x0000000000000000000000000000000000000000',
    to: overrides.to ?? '0x1111111111111111111111111111111111111111',
    value: overrides.value ?? 1234n,
    ethBlockNumber: overrides.ethBlockNumber ?? 0n,
  };
}

describe('verifyGlobalTeleportProofs', () => {
  it('accepts proofs whose Poseidon path matches the aggregation root', () => {
    const event = makeEvent();
    const leaf = computeLeafHash(event.to, event.value);
    const siblings = [
      asHex(computeLeafHash('0x2222222222222222222222222222222222222222', 42n)),
      asHex(computeLeafHash('0x3333333333333333333333333333333333333333', 7n)),
    ];
    const proof: GlobalTeleportProof = {
      siblings,
      leafIndex: 3n,
    };
    const root = computeMerkleRootFromSiblings({
      leaf,
      siblings: proof.siblings,
      leafIndex: proof.leafIndex,
    });
    expect(() =>
      verifyGlobalTeleportProofs({
        aggregationRoot: asHex(root),
        events: [event],
        proofs: [proof],
      }),
    ).not.toThrow();
  });

  it('throws when the recomputed root does not match', () => {
    const event = makeEvent();
    const leaf = computeLeafHash(event.to, event.value);
    const siblings = [
      asHex(computeLeafHash('0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1n)),
      asHex(computeLeafHash('0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 2n)),
    ];
    const proof: GlobalTeleportProof = {
      siblings,
      leafIndex: 1n,
    };
    const root = computeMerkleRootFromSiblings({
      leaf,
      siblings,
      leafIndex: proof.leafIndex,
    });
    const tamperedProof: GlobalTeleportProof = {
      ...proof,
      siblings: ['0x0', proof.siblings[1]],
    };
    expect(() =>
      verifyGlobalTeleportProofs({
        aggregationRoot: asHex(root),
        events: [event],
        proofs: [tamperedProof],
      }),
    ).toThrowError(/merkle proof mismatch/);
  });
});
