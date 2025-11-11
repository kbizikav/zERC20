import { describe, expect, test } from 'vitest';
import { Principal } from '@dfinity/principal';

import { addressToHex, authorizationMessage, deriveAddress, signAuthorization } from '../authorization.js';

const PRIVATE_KEY_ONE = (() => {
  const bytes = new Uint8Array(32);
  bytes[31] = 1;
  return bytes;
})();

const EXPECTED_ADDRESS_HEX = '0x7e5f4552091a69125d5dfcb7b8c2659029395bdf';

describe('authorization helpers', () => {
  test('deriveAddress matches known vector', () => {
    const address = deriveAddress(PRIVATE_KEY_ONE);
    expect(addressToHex(address)).toBe(EXPECTED_ADDRESS_HEX);
  });

  test('authorizationMessage encodes principal and address', () => {
    const address = deriveAddress(PRIVATE_KEY_ONE);
    const canisterId = Principal.fromText('aaaaa-aa');
    const transport = new Uint8Array(48);
    const message = authorizationMessage(canisterId, address, transport, 10n, 1n);
    const messageString = new TextDecoder().decode(message);
    expect(messageString).toContain(EXPECTED_ADDRESS_HEX);
    expect(messageString).toContain(canisterId.toText());
  });

  test('signAuthorization returns 65 byte signature', () => {
    const extended = authorizationMessage(Principal.fromText('aaaaa-aa'), new Uint8Array(20), new Uint8Array(48), 0n, 0n);
    const signature = signAuthorization(extended, PRIVATE_KEY_ONE);
    expect(signature.byteLength).toBe(65);
    expect(signature[64] === 27 || signature[64] === 28).toBe(true);
  });
});
