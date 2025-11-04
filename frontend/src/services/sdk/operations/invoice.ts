import { invoiceMessageText } from '../storage/invoice.js';
import { StealthCanisterClient } from '../storage/client.js';
import { InvoiceSubmission } from '../storage/types.js';
import { getBytes } from 'ethers';

import { NUM_BATCH_INVOICES } from '../core/constants.js';
import { InvoiceBatchBurnAddress, InvoiceIssueArtifacts } from '../core/types.js';
import {
  addressToBytes,
  ensureHexLength,
  hexFromBytes,
  normalizeHex,
} from '../core/utils.js';
import { buildFullBurnAddress, deriveInvoiceBatch, deriveInvoiceSingle } from '../wasm/index.js';

export interface InvoiceIssueParams {
  client: StealthCanisterClient;
  seedHex: string;
  recipientAddress: string;
  recipientChainId: number | bigint;
  isBatch: boolean;
}

const CHAIN_ID_OFFSET = 1;
const CHAIN_ID_LENGTH = 8;
const SEQUENCE_OFFSET = CHAIN_ID_OFFSET + CHAIN_ID_LENGTH;
const SEQUENCE_LENGTH = 32 - SEQUENCE_OFFSET;
const MAX_SEQUENCE_VALUE = (1n << BigInt(7 + SEQUENCE_LENGTH * 8)) - 1n;

function ensureSequenceBounds(sequence: bigint): void {
  if (sequence < 0n || sequence > MAX_SEQUENCE_VALUE) {
    throw new Error('invoice id sequence overflow');
  }
}

function applyInvoiceMode(bytes: Uint8Array, isBatch: boolean): void {
  if (isBatch) {
    bytes[0] &= 0x7f;
  } else {
    bytes[0] |= 0x80;
  }
}

function extractSequenceValue(invoiceBytes: Uint8Array): bigint {
  if (invoiceBytes.length !== 32) {
    throw new Error('invoice id must be 32 bytes');
  }
  let value = BigInt(invoiceBytes[0] & 0x7f);
  for (let index = SEQUENCE_OFFSET; index < invoiceBytes.length; index += 1) {
    value = (value << 8n) | BigInt(invoiceBytes[index]);
  }
  return value;
}

function applySequenceValue(invoiceBytes: Uint8Array, sequence: bigint): void {
  ensureSequenceBounds(sequence);
  let remaining = sequence;
  const tail = new Uint8Array(SEQUENCE_LENGTH);
  for (let index = SEQUENCE_LENGTH - 1; index >= 0; index -= 1) {
    tail[index] = Number(remaining & 0xffn);
    remaining >>= 8n;
  }
  const head = Number(remaining & 0x7fn);
  remaining >>= 7n;
  if (remaining !== 0n) {
    throw new Error('invoice id sequence overflow');
  }
  invoiceBytes[0] = (invoiceBytes[0] & 0x80) | head;
  for (let index = 0; index < SEQUENCE_LENGTH; index += 1) {
    invoiceBytes[SEQUENCE_OFFSET + index] = tail[index];
  }
}

function isSingleInvoiceBytes(invoiceBytes: Uint8Array): boolean {
  if (invoiceBytes.length !== 32) {
    throw new Error('invoice id must be 32 bytes');
  }
  return (invoiceBytes[0] & 0x80) !== 0;
}

function createInvoiceIdBytes(isBatch: boolean, chainId: bigint, sequence: bigint): Uint8Array {
  ensureSequenceBounds(sequence);
  const bytes = new Uint8Array(32);
  applyInvoiceMode(bytes, isBatch);
  embedChainId(bytes, chainId);
  applySequenceValue(bytes, sequence);
  return bytes;
}

export async function prepareInvoiceIssue(params: InvoiceIssueParams): Promise<InvoiceIssueArtifacts> {
  const {
    client,
    recipientAddress,
    recipientChainId,
    isBatch,
  } = params;
  const seedHex = ensureHexLength(params.seedHex, 32, 'seed');
  const normalizedChainId = BigInt(recipientChainId);

  const recipientBytes = addressToBytes(recipientAddress);
  const existing = await client.listInvoices(recipientBytes);
  const existingForChain = existing.filter(
    (bytes: Uint8Array) => extractChainIdFromInvoiceBytes(bytes) === normalizedChainId,
  );

  let maxSequence: bigint | undefined;
  for (const invoiceBytes of existingForChain) {
    const invoiceIsSingle = isSingleInvoiceBytes(invoiceBytes);
    if (invoiceIsSingle === !isBatch) {
      const sequence = extractSequenceValue(invoiceBytes);
      if (maxSequence === undefined || sequence > maxSequence) {
        maxSequence = sequence;
      }
    }
  }

  const nextSequence = (maxSequence ?? -1n) + 1n;
  const invoiceBytes = createInvoiceIdBytes(isBatch, normalizedChainId, nextSequence);
  const invoiceIdHex = hexFromBytes(invoiceBytes);

  const burnAddresses: InvoiceBatchBurnAddress[] = [];
  if (isBatch) {
    for (let subId = 0; subId < NUM_BATCH_INVOICES; subId += 1) {
      const secretAndTweak = await deriveInvoiceBatch(
        seedHex,
        invoiceIdHex,
        subId,
        normalizedChainId,
        recipientAddress,
      );
      const burn = await buildFullBurnAddress(
        normalizedChainId,
        recipientAddress,
        secretAndTweak.secret,
        secretAndTweak.tweak,
      );
      burnAddresses.push({
        subId,
        burnAddress: burn.burnAddress,
        secret: secretAndTweak.secret,
        tweak: secretAndTweak.tweak,
      });
    }
  } else {
    const secretAndTweak = await deriveInvoiceSingle(
      seedHex,
      invoiceIdHex,
      normalizedChainId,
      recipientAddress,
    );
    const burn = await buildFullBurnAddress(
      normalizedChainId,
      recipientAddress,
      secretAndTweak.secret,
      secretAndTweak.tweak,
    );
    burnAddresses.push({
      subId: 0,
      burnAddress: burn.burnAddress,
      secret: secretAndTweak.secret,
      tweak: secretAndTweak.tweak,
    });
  }

  const signatureMessage = invoiceMessageText(invoiceBytes);

  return {
    invoiceId: invoiceIdHex,
    recipientAddress: normalizeHex(recipientAddress),
    recipientChainId: normalizedChainId,
    burnAddresses,
    isBatch,
    signatureMessage,
  };
}

function embedChainId(bytes: Uint8Array, chainId: bigint): void {
  const normalizedChain = BigInt.asUintN(64, chainId);
  for (let index = 0; index < CHAIN_ID_LENGTH; index += 1) {
    const shift = BigInt(8 * (CHAIN_ID_LENGTH - 1 - index));
    const nextByte = Number((normalizedChain >> shift) & 0xffn);
    bytes[CHAIN_ID_OFFSET + index] = nextByte;
  }
}

export function extractChainIdFromInvoiceBytes(invoiceBytes: Uint8Array): bigint {
  if (invoiceBytes.length !== 32) {
    throw new Error('invoice id must be 32 bytes');
  }
  let value = 0n;
  for (let index = 0; index < CHAIN_ID_LENGTH; index += 1) {
    value = (value << 8n) | BigInt(invoiceBytes[CHAIN_ID_OFFSET + index]);
  }
  return value;
}

export function extractChainIdFromInvoiceHex(invoiceIdHex: string): bigint {
  const bytes = getBytes(ensureHexLength(invoiceIdHex, 32, 'invoice id'));
  return extractChainIdFromInvoiceBytes(bytes);
}

export function isSingleInvoiceHex(invoiceIdHex: string): boolean {
  const bytes = getBytes(ensureHexLength(invoiceIdHex, 32, 'invoice id'));
  return isSingleInvoiceBytes(bytes);
}

export async function submitInvoice(
  client: StealthCanisterClient,
  invoiceIdHex: string,
  signature: Uint8Array,
): Promise<void> {
  const invoiceBytes = getBytes(ensureHexLength(invoiceIdHex, 32, 'invoice id'));
  const submission: InvoiceSubmission = {
    invoiceId: invoiceBytes,
    signature,
  };
  await client.submitInvoice(submission);
}

export async function listInvoices(
  client: StealthCanisterClient,
  ownerAddress: string,
  chainId?: number | bigint,
): Promise<string[]> {
  const ownerBytes = addressToBytes(ownerAddress);
  const response = await client.listInvoices(ownerBytes);
  const normalizedChainId = chainId === undefined ? undefined : BigInt(chainId);
  return response
    .filter((bytes: Uint8Array) =>
      normalizedChainId === undefined || extractChainIdFromInvoiceBytes(bytes) === normalizedChainId,
    )
    .map((bytes: Uint8Array) => hexFromBytes(bytes));
}
