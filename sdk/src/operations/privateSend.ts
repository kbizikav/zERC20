import { encryptAnnouncementWithArtifacts } from '../ic/encryption.js';
import { StealthCanisterClient } from '../ic/client.js';
import { Announcement } from '../ic/types.js';

import { PreparedPrivateSend, PrivateSendResult } from '../types.js';
import {
  addressToBytes,
  bytesToHex,
  ensureHexLength,
  hexToBytes,
  normalizeHex,
  randomBytes,
} from '../utils/hex.js';
import { buildFullBurnAddress, derivePaymentAdvice } from '../wasm/index.js';

export interface PreparePrivateSendParams {
  client: StealthCanisterClient;
  recipientAddress: string;
  recipientChainId: number | bigint;
  seedHex: string;
  paymentAdviceIdHex?: string;
  randomBytes?: (length: number) => Uint8Array;
}

export async function preparePrivateSend(
  params: PreparePrivateSendParams,
): Promise<PreparedPrivateSend> {
  const { client, recipientAddress, recipientChainId } = params;
  const seedHex = ensureHexLength(params.seedHex, 32, 'seed');

  let paymentAdviceBytes: Uint8Array;
  if (params.paymentAdviceIdHex) {
    const normalized = ensureHexLength(params.paymentAdviceIdHex, 32, 'payment advice id');
    paymentAdviceBytes = hexToBytes(normalized);
  } else {
    const rng = params.randomBytes ?? randomBytes;
    paymentAdviceBytes = rng(32);
  }
  const paymentAdviceIdHex = bytesToHex(paymentAdviceBytes);

  const secretAndTweak = await derivePaymentAdvice(
    seedHex,
    paymentAdviceIdHex,
    recipientChainId,
    recipientAddress,
  );
  const burnArtifacts = await buildFullBurnAddress(
    recipientChainId,
    recipientAddress,
    secretAndTweak.secret,
    secretAndTweak.tweak,
  );

  const recipientBytes = addressToBytes(recipientAddress);
  const viewPublicKey = await client.getViewPublicKey(recipientBytes);
  const burnPayloadBytes = hexToBytes(normalizeHex(burnArtifacts.fullBurnAddress));
  const { announcement, sessionKey } = await encryptAnnouncementWithArtifacts(viewPublicKey, burnPayloadBytes);

  return {
    paymentAdviceId: paymentAdviceIdHex,
    paymentAdviceIdBytes: paymentAdviceBytes,
    secret: secretAndTweak.secret,
    tweak: secretAndTweak.tweak,
    burnAddress: burnArtifacts.burnAddress,
    burnPayload: burnArtifacts.fullBurnAddress,
    generalRecipient: burnArtifacts.generalRecipient,
    announcement,
    sessionKey,
  };
}

export interface SubmitPrivateSendParams {
  client: StealthCanisterClient;
  preparation: PreparedPrivateSend;
}

export async function submitPrivateSendAnnouncement(
  params: SubmitPrivateSendParams,
): Promise<PrivateSendResult> {
  const { client, preparation } = params;
  const storedAnnouncement: Announcement = await client.submitAnnouncement(preparation.announcement);
  return {
    paymentAdviceId: preparation.paymentAdviceId,
    secret: preparation.secret,
    tweak: preparation.tweak,
    burnAddress: preparation.burnAddress,
    burnPayload: preparation.burnPayload,
    generalRecipient: preparation.generalRecipient,
    announcement: storedAnnouncement,
  };
}
