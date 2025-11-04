import type { Announcement, AnnouncementInput } from '../storage/types.js';

export interface SecretAndTweak {
  secret: string;
  tweak: string;
}

export interface GeneralRecipientArtifact {
  chainId: bigint;
  address: string;
  tweak: string;
  fr: string;
  u256: string;
}

export interface BurnArtifacts {
  burnAddress: string;
  fullBurnAddress: string;
  secret: string;
  tweak: string;
  generalRecipient: GeneralRecipientArtifact;
}

export interface PrivateSendPreparation {
  paymentAdviceId: string;
  secret: string;
  tweak: string;
  burnAddress: string;
  burnPayload: string;
  generalRecipient: GeneralRecipientArtifact;
}

export interface AnnouncementArtifacts {
  announcement: AnnouncementInput;
  sessionKey: Uint8Array;
}

export interface PreparedPrivateSend extends PrivateSendPreparation, AnnouncementArtifacts {
  paymentAdviceIdBytes: Uint8Array;
}

export interface PrivateSendResult {
  paymentAdviceId: string;
  secret: string;
  tweak: string;
  burnAddress: string;
  burnPayload: string;
  generalRecipient: GeneralRecipientArtifact;
  announcement: Announcement;
}

export interface InvoiceBatchBurnAddress {
  subId: number;
  burnAddress: string;
  secret: string;
  tweak: string;
}

export interface InvoiceIssueArtifacts {
  invoiceId: string;
  recipientAddress: string;
  recipientChainId: bigint;
  burnAddresses: InvoiceBatchBurnAddress[];
  signatureMessage: string;
}

export interface ScanInvoicesResult {
  invoiceIds: string[];
}

export interface ScannedAnnouncement {
  id: bigint;
  burnAddress: string;
  fullBurnAddress: string;
  createdAtNs: bigint;
  recipientChainId: bigint;
}
