import { bytesToHex } from '../utils/hex.js';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function invoiceMessageText(invoiceId: Uint8Array): string {
  if (invoiceId.length !== 32) {
    throw new Error('invoiceId must be 32 bytes');
  }
  return `ICP Stealth Invoice Submission:\ninvoice_id: 0x${bytesToHex(invoiceId)}`;
}

export function invoiceMessage(invoiceId: Uint8Array): Uint8Array {
  const message = invoiceMessageText(invoiceId);
  return eip191Message(encoder.encode(message));
}

export function invoiceMessageToString(message: Uint8Array): string {
  return decoder.decode(message);
}

function eip191Message(message: Uint8Array): Uint8Array {
  const prefix = `\x19Ethereum Signed Message:\n${message.length}`;
  const prefixBytes = encoder.encode(prefix);
  const result = new Uint8Array(prefixBytes.length + message.length);
  result.set(prefixBytes, 0);
  result.set(message, prefixBytes.length);
  return result;
}
