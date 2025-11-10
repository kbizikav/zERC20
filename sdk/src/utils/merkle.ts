import poseidon from 'poseidon-lite';

import type { GlobalTeleportProof, IndexedEvent } from '../types.js';
import { normalizeHex, toBigInt } from './hex.js';

const BN254_FIELD_MODULUS = BigInt('0x30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd47');

function ensureFieldElement(value: bigint, label: string): bigint {
  if (value < 0n) {
    throw new Error(`${label} must be non-negative`);
  }
  if (value >= BN254_FIELD_MODULUS) {
    throw new Error(`${label} exceeds the BN254 field modulus`);
  }
  return value;
}

function hexToField(value: string, label: string): bigint {
  const normalized = normalizeHex(value);
  return ensureFieldElement(BigInt(normalized), label);
}

function intoField(value: bigint | number | string, label: string): bigint {
  const bigIntValue = typeof value === 'bigint' ? value : toBigInt(value);
  return ensureFieldElement(bigIntValue, label);
}

function poseidon2(left: bigint, right: bigint): bigint {
  const result = poseidon([left, right]);
  if (typeof result !== 'bigint') {
    throw new Error('poseidon-lite returned a non-bigint result');
  }
  return ensureFieldElement(result, 'poseidon result');
}

export function computeLeafHash(addressHex: string, value: bigint | number | string): bigint {
  const addressField = hexToField(addressHex, 'leaf address');
  const valueField = intoField(value, 'leaf value');
  return poseidon2(addressField, valueField);
}

export function computeMerkleRootFromSiblings(params: {
  leaf: bigint;
  siblings: readonly string[];
  leafIndex: bigint | number;
}): bigint {
  let state = ensureFieldElement(params.leaf, 'leaf');
  let index = toBigInt(params.leafIndex);
  if (index < 0n) {
    throw new Error('leafIndex must be non-negative');
  }

  params.siblings.forEach((siblingHex, level) => {
    const sibling = hexToField(siblingHex, `siblings[${level}]`);
    if ((index & 1n) === 1n) {
      state = poseidon2(sibling, state);
    } else {
      state = poseidon2(state, sibling);
    }
    index >>= 1n;
  });

  return state;
}

export function verifyGlobalTeleportProofs(args: {
  aggregationRoot: string;
  events: readonly IndexedEvent[];
  proofs: readonly GlobalTeleportProof[];
}): void {
  const { aggregationRoot, events, proofs } = args;
  if (events.length !== proofs.length) {
    throw new Error('events length must match proofs length for global teleport verification');
  }
  if (events.length === 0) {
    return;
  }

  const expectedRoot = hexToField(aggregationRoot, 'aggregationRoot');

  for (let idx = 0; idx < proofs.length; idx++) {
    const proof = proofs[idx];
    const event = events[idx];
    const leaf = computeLeafHash(event.to, event.value);
    const derivedRoot = computeMerkleRootFromSiblings({
      leaf,
      siblings: proof.siblings,
      leafIndex: proof.leafIndex,
    });
    if (derivedRoot !== expectedRoot) {
      const leafLabel = `leafIndex ${proof.leafIndex.toString()} (eventIndex ${event.eventIndex.toString()})`;
      throw new Error(`merkle proof mismatch for ${leafLabel}`);
    }
  }
}

export { BN254_FIELD_MODULUS };
