import { GLOBAL_TRANSFER_TREE_HEIGHT, NUM_BATCH_INVOICES } from '../constants.js';
import { normalizeHex } from '../utils/hex.js';

export function toFixedHex(value: bigint, bytes: number = 32): string {
  if (value < 0n) {
    throw new Error('toFixedHex: negative values are not supported');
  }
  const hex = value.toString(16).padStart(bytes * 2, '0');
  return `0x${hex}`;
}

export function toFieldHex(value: bigint): string {
  return toFixedHex(value, 32);
}

export function toLeafIndexString(index: bigint): string {
  return index.toString(10);
}

function zeroPadField(value: string, label: string): string {
  const normalized = normalizeHex(value);
  const body = normalized.slice(2);
  if (!/^[0-9a-f]*$/i.test(body)) {
    throw new Error(`${label} contains non-hex characters: ${normalized}`);
  }
  if (body.length % 2 !== 0) {
    throw new Error(`${label} must have an even number of hex digits: ${normalized}`);
  }
  if (body.length > 64) {
    throw new Error(`${label} must fit within 32 bytes: ${normalized}`);
  }
  return `0x${body.padStart(64, '0')}`;
}

export function formatFieldElement(value: string, label: string): string {
  return zeroPadField(value, label);
}

export function randomDummySteps(): number {
  const buffer = new Uint32Array(1);
  if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
    crypto.getRandomValues(buffer);
    const value = buffer[0] % NUM_BATCH_INVOICES;
    return value === 0 ? 1 : value;
  }
  return 1;
}

export function appendDummySteps(steps: any[]): void {
  const dummySteps = randomDummySteps();
  const maxLeaves = 1n << BigInt(30);
  const offset = maxLeaves - 1n - BigInt(dummySteps);
  for (let i = 0; i < dummySteps; i += 1) {
    const leafIndex = offset + BigInt(i);
    steps.push({
      is_dummy: true,
      value: formatFieldElement('0x0', `dummySteps[${i}].value`),
      secret: formatFieldElement('0x0', `dummySteps[${i}].secret`),
      leafIndex: toLeafIndexString(leafIndex),
      siblings: Array(GLOBAL_TRANSFER_TREE_HEIGHT)
        .fill(null)
        .map((_, siblingIdx) =>
          formatFieldElement('0x0', `dummySteps[${i}].siblings[${siblingIdx}]`),
        ),
    });
  }
}
