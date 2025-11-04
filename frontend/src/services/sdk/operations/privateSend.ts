import { encryptAnnouncementWithArtifacts } from '../storage/encryption.js';
import { StealthCanisterClient } from '../storage/client.js';
import { Announcement } from '../storage/types.js';
import { getBytes } from 'ethers';

import {
  PreparedPrivateSend,
  PrivateSendResult,
} from '../core/types.js';
import {
  addressToBytes,
  ensureHexLength,
  hexFromBytes,
  normalizeHex,
  randomBytes,
} from '../core/utils.js';
import {
  buildFullBurnAddress,
  derivePaymentAdvice,
  getSeedMessage,
} from '../wasm/index.js';

export interface PreparePrivateSendParams {
  client: StealthCanisterClient;
  recipientAddress: string;
  recipientChainId: number | bigint;
  seedHex: string;
  paymentAdviceIdHex?: string;
  randomBytes?: (length: number) => Uint8Array;
}

export async function seedDerivationMessage(): Promise<string> {
  return getSeedMessage();
}

export async function preparePrivateSend(
  params: PreparePrivateSendParams,
): Promise<PreparedPrivateSend> {
  const { client, recipientAddress, recipientChainId } = params;
  const seedHex = ensureHexLength(params.seedHex, 32, 'seed');

  let paymentAdviceBytes: Uint8Array;
  if (params.paymentAdviceIdHex) {
    const normalized = ensureHexLength(params.paymentAdviceIdHex, 32, 'payment advice id');
    paymentAdviceBytes = getBytes(normalized);
  } else {
    const rng = params.randomBytes ?? randomBytes;
    paymentAdviceBytes = rng(32);
  }
  const paymentAdviceIdHex = hexFromBytes(paymentAdviceBytes);

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
  const burnPayloadBytes = getBytes(normalizeHex(burnArtifacts.fullBurnAddress));
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
