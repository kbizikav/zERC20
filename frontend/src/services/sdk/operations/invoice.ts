import { invoiceMessageText } from '../storage/invoice.js';
import { StealthCanisterClient } from '../storage/client.js';
import { InvoiceSubmission } from '../storage/types.js';
import { getBytes } from 'ethers';

import { NUM_BATCH_INVOICES } from '../core/constants.js';
import {
  InvoiceBatchBurnAddress,
  InvoiceIssueArtifacts,
} from '../core/types.js';
import {
  addressToBytes,
  ensureHexLength,
  hexFromBytes,
  normalizeHex,
  randomBytes,
} from '../core/utils.js';
import { buildFullBurnAddress, deriveInvoiceBatch, deriveInvoiceSingle } from '../wasm/index.js';

export interface InvoiceIssueParams {
  client: StealthCanisterClient;
  seedHex: string;
  recipientAddress: string;
  recipientChainId: number | bigint;
  isBatch: boolean;
  randomBytes?: (length: number) => Uint8Array;
  maxRetries?: number;
}

export async function prepareInvoiceIssue(params: InvoiceIssueParams): Promise<InvoiceIssueArtifacts> {
  const {
    client,
    recipientAddress,
    recipientChainId,
    isBatch,
    randomBytes: rng,
    maxRetries = 8,
  } = params;
  const seedHex = ensureHexLength(params.seedHex, 32, 'seed');
  const randomFn = rng ?? randomBytes;
  const normalizedChainId = BigInt(recipientChainId);

  const recipientBytes = addressToBytes(recipientAddress);
  const existing = await client.listInvoices(recipientBytes);
  const existingIds = new Set(
    existing
      .filter((bytes: Uint8Array) => extractChainIdFromInvoiceBytes(bytes) === normalizedChainId)
      .map((bytes: Uint8Array) => hexFromBytes(bytes)),
  );

  let invoiceBytes: Uint8Array | null = null;
  for (let attempt = 0; attempt < maxRetries; attempt += 1) {
    const candidate = generateInvoiceIdBytes(isBatch, normalizedChainId, randomFn);
    const candidateHex = hexFromBytes(candidate);
    if (!existingIds.has(candidateHex)) {
      invoiceBytes = candidate;
      break;
    }
  }
  if (!invoiceBytes) {
    throw new Error('failed to generate unique invoice id');
  }
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
    signatureMessage,
  };
}

const CHAIN_ID_OFFSET = 1;
const CHAIN_ID_LENGTH = 8;

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

function generateInvoiceIdBytes(
  isBatch: boolean,
  chainId: bigint,
  rng: (length: number) => Uint8Array,
): Uint8Array {
  const bytes = rng(32);
  if (bytes.length !== 32) {
    throw new Error('invoice id generator must return 32 bytes');
  }
  if (isBatch) {
    bytes[0] &= 0x7f;
  } else {
    bytes[0] |= 0x80;
  }
  embedChainId(bytes, chainId);
  return bytes;
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
