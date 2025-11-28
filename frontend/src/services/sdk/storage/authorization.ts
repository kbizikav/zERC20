import { Principal } from '@dfinity/principal';
import { SigningKey, computeAddress, getBytes, hexlify, keccak256 } from 'ethers';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export type Address = Uint8Array;

export function deriveAddress(privateKey: Uint8Array): Address {
  if (privateKey.length !== 32) {
    throw new Error('private key must be 32 bytes');
  }
  const privateKeyHex = hexlify(privateKey);
  const addressHex = computeAddress(privateKeyHex);
  return hexToBytes(addressHex);
}

export function authorizationMessageText(
  canisterId: Principal,
  address: Address,
  transportPublicKey: Uint8Array,
  expiryNs: bigint,
  nonce: bigint,
): string {
  return `ICP Stealth Authorization:\naddress: 0x${bytesToHex(address)}\ncanister: ${canisterId.toText()}\ntransport: 0x${bytesToHex(transportPublicKey)}\nexpiry_ns:${expiryNs}\nnonce:${nonce}`;
}

export function authorizationMessage(
  canisterId: Principal,
  address: Address,
  transportPublicKey: Uint8Array,
  expiryNs: bigint,
  nonce: bigint,
): Uint8Array {
  return eip191Message(encoder.encode(authorizationMessageText(canisterId, address, transportPublicKey, expiryNs, nonce)));
}

export function signAuthorization(message: Uint8Array, privateKey: Uint8Array): Uint8Array {
  if (message.length === 0) {
    throw new Error('message must not be empty');
  }
  if (privateKey.length !== 32) {
    throw new Error('private key must be 32 bytes');
  }
  const digestHex = keccak256(message);
  const signingKey = new SigningKey(hexlify(privateKey));
  const signature = signingKey.sign(digestHex);
  return getBytes(signature.serialized);
}

export function unixTimeNs(): bigint {
  const nowMs = BigInt(Date.now());
  return nowMs * 1_000_000n;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export function hexToBytes(hex: string): Uint8Array {
  const normalized = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (normalized.length % 2 !== 0) {
    throw new Error('hex string must have even length');
  }
  const bytes = new Uint8Array(normalized.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(normalized.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function eip191Message(message: Uint8Array): Uint8Array {
  const prefix = `\x19Ethereum Signed Message:\n${message.length}`;
  const prefixBytes = encoder.encode(prefix);
  const result = new Uint8Array(prefixBytes.length + message.length);
  result.set(prefixBytes, 0);
  result.set(message, prefixBytes.length);
  return result;
}

export function addressToHex(address: Address): string {
  return `0x${bytesToHex(address)}`;
}

export function messageToString(message: Uint8Array): string {
  return decoder.decode(message);
}
