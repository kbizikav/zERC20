export interface AnnouncementInput {
  ibeCiphertext: Uint8Array;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
}

export interface Announcement {
  id: bigint;
  ibeCiphertext: Uint8Array;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
  createdAtNs: bigint;
}

export interface AnnouncementPage {
  announcements: Announcement[];
  nextId: bigint | null;
}

export interface InvoiceSubmission {
  invoiceId: Uint8Array;
  signature: Uint8Array;
}

export interface DecryptedAnnouncement {
  id: bigint;
  plaintext: Uint8Array;
  createdAtNs: bigint;
}

export interface EncryptedViewKeyRequest {
  address: Uint8Array;
  transportPublicKey: Uint8Array;
  expiryNs: bigint;
  nonce: bigint;
  signature: Uint8Array;
}

export interface EncryptedViewKeyResponse {
  encryptedKey: Uint8Array;
  viewPublicKey: Uint8Array;
}

export type CanisterResult<T> =
  | { Ok: T }
  | { Err: string }
  | { ok: T }
  | { err: string };

export interface VetKdConfig {
  /** Domain separator passed to the key manager when deriving keys */
  context: Uint8Array;
  /** VetKD key name (e.g. `key_1`, `test_key_1`, `dfx_test_key`). */
  keyIdName: string;
}

export interface StealthClientOptions {
  storageCanisterIdText: string;
  keyManagerCanisterIdText: string;
}

export interface EncryptOptions {
  identity?: Uint8Array;
  rng?: () => Uint8Array;
}

export interface EncryptionArtifacts {
  announcement: AnnouncementInput;
  sessionKey: Uint8Array;
}
