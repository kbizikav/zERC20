import { Principal } from '@dfinity/principal';
import { secp256k1 } from '@noble/curves/secp256k1';
import { keccak256, toHex } from 'viem';
import { privateKeyToAccount } from 'viem/accounts';

import { bytesToHex, hexToBytes } from '../utils/hex.js';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export type Address = Uint8Array;

export function deriveAddress(privateKey: Uint8Array): Address {
  if (privateKey.length !== 32) {
    throw new Error('private key must be 32 bytes');
  }
  const account = privateKeyToAccount(toHex(privateKey));
  return hexToBytes(account.address);
}

export function authorizationMessageText(
  canisterId: Principal,
  address: Address,
  transportPublicKey: Uint8Array,
  expiryNs: bigint,
  nonce: bigint,
): string {
  return `ICP Stealth Authorization:\naddress: ${bytesToHex(address)}\ncanister: ${canisterId.toText()}\ntransport: ${bytesToHex(transportPublicKey)}\nexpiry_ns:${expiryNs}\nnonce:${nonce}`;
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
  const digestBytes = hexToBytes(digestHex);
  const signature = secp256k1.sign(digestBytes, privateKey);
  const serialized = new Uint8Array(65);
  serialized.set(signature.toCompactRawBytes(), 0);
  serialized[64] = signature.recovery + 27;
  return serialized;
}

export function unixTimeNs(): bigint {
  const nowMs = BigInt(Date.now());
  return nowMs * 1_000_000n;
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
  return bytesToHex(address);
}

export function messageToString(message: Uint8Array): string {
  return decoder.decode(message);
}
