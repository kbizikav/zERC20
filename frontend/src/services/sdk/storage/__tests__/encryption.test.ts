import { beforeAll, describe, expect, test } from 'vitest';
import { webcrypto } from 'node:crypto';
import { Principal } from '@dfinity/principal';
import { MasterPublicKey, MasterPublicKeyId } from '@dfinity/vetkeys';

import { deriveContext } from '../config.js';
import { encryptAnnouncement } from '../encryption.js';

beforeAll(() => {
  if (typeof globalThis.crypto === 'undefined') {
    (globalThis as any).crypto = webcrypto;
  }
});

describe('encryption helpers', () => {
  test('encryptAnnouncement produces ciphertext and nonce', async () => {
    const address = new Uint8Array(20);
    const context = deriveContext(address);
    const masterKey = MasterPublicKey.productionKey(MasterPublicKeyId.TEST_KEY_1);
    const canisterPrincipal = Principal.fromText('3gshj-ayaaa-aaaac-a4wfq-cai');
    const derived = masterKey
      .deriveCanisterKey(canisterPrincipal.toUint8Array())
      .deriveSubKey(context);
    const viewPublicKey = derived.publicKeyBytes();

    const announcement = await encryptAnnouncement(viewPublicKey, 'hello world');

    expect(announcement.ciphertext.byteLength).toBeGreaterThan(0);
    expect(announcement.nonce.byteLength).toBe(12);
    expect(announcement.ibeCiphertext.byteLength).toBeGreaterThan(announcement.ciphertext.byteLength);
  });
});
