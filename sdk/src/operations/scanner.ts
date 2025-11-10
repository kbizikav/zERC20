import { authorizationMessage, authorizationMessageText, unixTimeNs } from '../ic/authorization.js';
import { StealthCanisterClient } from '../ic/client.js';
import { decryptVetKey, prepareTransportKey, TransportKeyPair } from '../ic/recipient.js';
import { scanAnnouncements } from '../ic/encryption.js';
import { Announcement, EncryptedViewKeyRequest } from '../ic/types.js';
import type { VetKey } from '@dfinity/vetkeys';
import { ScannedAnnouncement } from '../types.js';
import { addressToBytes, bytesToHex } from '../utils/hex.js';
import { decodeFullBurnAddress } from '../wasm/index.js';

export interface AuthorizationPayload {
  message: string;
  canonicalMessage: Uint8Array;
  expiryNs: bigint;
  nonce: bigint;
  transport: TransportKeyPair;
}

export async function createAuthorizationPayload(
  client: StealthCanisterClient,
  address: string,
  ttlSeconds: number = 60,
): Promise<AuthorizationPayload> {
  const addressBytes = addressToBytes(address);
  const transport = prepareTransportKey();
  const nowNs = unixTimeNs();
  const expiryNs = nowNs + BigInt(ttlSeconds) * 1_000_000_000n;
  const maxNonce = await client.getMaxNonce(addressBytes);
  const nonce = maxNonce + 1n;
  const message = authorizationMessageText(
    client.getKeyManagerCanisterId(),
    addressBytes,
    transport.publicKey,
    expiryNs,
    nonce,
  );
  const canonicalMessage = authorizationMessage(
    client.getKeyManagerCanisterId(),
    addressBytes,
    transport.publicKey,
    expiryNs,
    nonce,
  );
  return { message, canonicalMessage, expiryNs, nonce, transport };
}

export async function requestVetKey(
  client: StealthCanisterClient,
  address: string,
  payload: AuthorizationPayload,
  signature: Uint8Array,
): Promise<VetKey> {
  const addressBytes = addressToBytes(address);
  const request: EncryptedViewKeyRequest = {
    address: addressBytes,
    transportPublicKey: payload.transport.publicKey,
    expiryNs: payload.expiryNs,
    nonce: payload.nonce,
    signature,
  };
  const encrypted = await client.requestEncryptedViewKey(request);
  return decryptVetKey(encrypted, await client.getViewPublicKey(addressBytes), payload.transport.secret);
}

export interface ScanReceivingsParams {
  client: StealthCanisterClient;
  vetKey: VetKey;
  pageSize?: number;
  startAfter?: bigint;
}

export async function scanReceivings(
  params: ScanReceivingsParams,
): Promise<ScannedAnnouncement[]> {
  const { client, vetKey, pageSize = 100, startAfter } = params;
  let cursor = startAfter ?? 0n;
  const collected: ScannedAnnouncement[] = [];

  while (true) {
    const page = await client.listAnnouncements(cursor === 0n ? undefined : cursor, pageSize);
    if (page.announcements.length === 0) {
      break;
    }
    const decrypted = await scanAnnouncements(vetKey, page.announcements);
    for (const item of decrypted) {
      try {
        const burnArtifacts = await decodeFullBurnAddress(bytesToHex(item.plaintext));
        collected.push({
          id: item.id,
          burnAddress: burnArtifacts.burnAddress,
          fullBurnAddress: burnArtifacts.fullBurnAddress,
          createdAtNs: item.createdAtNs,
          recipientChainId: burnArtifacts.generalRecipient.chainId,
        });
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        // Ignore announcements that fail PoW recovery and continue scanning.
        const lower = message.toLowerCase();
        if (lower.includes('pow') || lower.includes('derive burn address')) {
          continue;
        }
        throw err;
      }
    }
    const last = page.announcements[page.announcements.length - 1];
    cursor = last.id;
    if (page.nextId === null) {
      break;
    }
  }

  return collected;
}
