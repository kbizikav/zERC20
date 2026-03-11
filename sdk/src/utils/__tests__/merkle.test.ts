import { describe, expect, it } from 'vitest';

import type { GlobalTeleportProof, IndexedEvent, LocalTeleportProof } from '../../types.js';
import {
  computeLeafHash,
  computeMerkleRootFromSiblings,
  verifyTeleportProofs,
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

describe('verifyTeleportProofs', () => {
  it('accepts proofs whose Poseidon path matches the aggregation root', () => {
    const event = makeEvent();
    const leaf = computeLeafHash(event.to, event.value);
    const siblings = [
      asHex(computeLeafHash('0x2222222222222222222222222222222222222222', 42n)),
      asHex(computeLeafHash('0x3333333333333333333333333333333333333333', 7n)),
    ];
    const proof: GlobalTeleportProof = {
      kind: 'global',
      siblings,
      leafIndex: 3n,
    };
    const root = computeMerkleRootFromSiblings({
      leaf,
      siblings: proof.siblings,
      leafIndex: proof.leafIndex,
    });
    expect(() =>
      verifyTeleportProofs({
        scope: 'global',
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
      kind: 'global',
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
      verifyTeleportProofs({
        scope: 'global',
        aggregationRoot: asHex(root),
        events: [event],
        proofs: [tamperedProof],
      }),
    ).toThrowError(/merkle proof mismatch/);
  });

  it('accepts local proofs whose Poseidon path matches the local aggregation root', () => {
    const event = makeEvent({ eventIndex: 9n, value: 77n });
    const leaf = computeLeafHash(event.to, event.value);
    const siblings = [
      asHex(computeLeafHash('0x4444444444444444444444444444444444444444', 9n)),
      asHex(computeLeafHash('0x5555555555555555555555555555555555555555', 10n)),
    ];
    const proof: LocalTeleportProof = {
      kind: 'local',
      treeIndex: 2n,
      event,
      siblings,
    };
    const root = computeMerkleRootFromSiblings({
      leaf,
      siblings: proof.siblings,
      leafIndex: proof.treeIndex,
    });

    expect(() =>
      verifyTeleportProofs({
        scope: 'local',
        aggregationRoot: asHex(root),
        events: [event],
        proofs: [proof],
      }),
    ).not.toThrow();
  });
});
