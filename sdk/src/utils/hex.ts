import { bytesToHex as viemBytesToHex, hexToBytes as viemHexToBytes } from 'viem';

export type HexLike =
  | string
  | number
  | bigint
  | Uint8Array
  | { toHexString(): string }
  | { _hex: string };

function ensureEvenHex(hex: string): string {
  if (hex.length === 0) {
    return '00';
  }
  return hex.length % 2 === 0 ? hex : `0${hex}`;
}

export function bytesToHex(bytes: Uint8Array): string {
  return viemBytesToHex(bytes);
}

export function normalizeHex(value: HexLike): string {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (trimmed.length === 0) {
      throw new Error('normalizeHex: empty string provided');
    }
    const lower = trimmed.startsWith('0X') ? `0x${trimmed.slice(2)}` : trimmed;
    if (lower.startsWith('0x')) {
      return `0x${ensureEvenHex(lower.slice(2).toLowerCase())}`;
    }
    return `0x${ensureEvenHex(lower.toLowerCase())}`;
  }

  if (typeof value === 'bigint') {
    return normalizeHex(value.toString(16));
  }

  if (typeof value === 'number') {
    if (!Number.isInteger(value)) {
      throw new Error(`normalizeHex: expected integer number, received ${value}`);
    }
    return normalizeHex(value.toString(16));
  }

  if (value instanceof Uint8Array) {
    return bytesToHex(value);
  }

  if (value && typeof value === 'object') {
    if (typeof (value as { toHexString?: unknown }).toHexString === 'function') {
      return normalizeHex((value as { toHexString(): string }).toHexString());
    }
    if (typeof (value as { _hex?: unknown })._hex === 'string') {
      return normalizeHex((value as { _hex: string })._hex);
    }
  }

  throw new Error('normalizeHex: unsupported value type');
}

export function hexToBytes(value: string): Uint8Array {
  const normalized = normalizeHex(value) as `0x${string}`;
  if (normalized === '0x') {
    return new Uint8Array();
  }
  return viemHexToBytes(normalized);
}

export function ensureHexLength(value: HexLike, expectedBytes: number, label: string): string {
  const normalized = normalizeHex(value);
  const bytes = hexToBytes(normalized);
  if (bytes.length !== expectedBytes) {
    throw new Error(`${label} must be ${expectedBytes} bytes, received ${bytes.length}`);
  }
  return normalized;
}

export function addressToBytes(address: string): Uint8Array {
  const normalized = ensureHexLength(address, 20, 'address');
  return hexToBytes(normalized);
}

export function randomBytes(length: number): Uint8Array {
  const buffer = new Uint8Array(length);
  if (typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function') {
    crypto.getRandomValues(buffer);
    return buffer;
  }
  throw new Error('secure randomness is not available (missing global crypto)');
}

export function toBigInt(value: number | string | bigint): bigint {
  if (typeof value === 'bigint') {
    return value;
  }
  if (typeof value === 'number') {
    if (!Number.isInteger(value)) {
      throw new Error(`toBigInt: expected integer number, received ${value}`);
    }
    return BigInt(value);
  }
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (trimmed.length === 0) {
      throw new Error('toBigInt: empty string provided');
    }
    if (trimmed.startsWith('0x') || trimmed.startsWith('0X')) {
      return BigInt(trimmed);
    }
    return BigInt(trimmed);
  }
  throw new Error('toBigInt: unsupported value type');
}
