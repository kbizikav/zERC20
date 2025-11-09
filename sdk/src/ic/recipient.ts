import {
  DerivedPublicKey,
  EncryptedVetKey,
  TransportSecretKey,
  VetKey,
} from '@dfinity/vetkeys';

import { StealthError } from './errors.js';

export interface TransportKeyPair {
  secret: TransportSecretKey;
  publicKey: Uint8Array;
}

export function prepareTransportKey(): TransportKeyPair {
  const secret = TransportSecretKey.random();
  const publicKey = secret.publicKeyBytes();
  return { secret, publicKey };
}

export function decryptVetKey(
  encryptedKey: Uint8Array,
  viewPublicKey: Uint8Array,
  transportSecret: TransportSecretKey,
  identity: Uint8Array = new Uint8Array(),
): VetKey {
  try {
    const encrypted = EncryptedVetKey.deserialize(encryptedKey);
    const derived = DerivedPublicKey.deserialize(viewPublicKey);
    return encrypted.decryptAndVerify(transportSecret, derived, identity);
  } catch (error) {
    if (error instanceof StealthError) {
      throw error;
    }
    throw new StealthError('failed to decrypt vet key');
  }
}
