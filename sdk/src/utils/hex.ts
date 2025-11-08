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

function bytesToHex(bytes: Uint8Array): string {
  if (bytes.length === 0) {
    return '0x';
  }
  const chunks = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0'));
  return `0x${chunks.join('')}`;
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
  const normalized = normalizeHex(value).slice(2);
  if (normalized.length === 0) {
    return new Uint8Array();
  }
  const bytes = new Uint8Array(normalized.length / 2);
  for (let i = 0; i < normalized.length; i += 2) {
    bytes[i / 2] = parseInt(normalized.slice(i, i + 2), 16);
  }
  return bytes;
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
