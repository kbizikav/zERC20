const DEFAULT_DOMAIN = 'icp-stealth-announcement-v1';

const encoder = new TextEncoder();

export const DEFAULT_CONTEXT_PREFIX = encoder.encode(DEFAULT_DOMAIN);
export const DEFAULT_KEY_ID_NAME = 'key_1';
export const DEFAULT_TEST_KEY_ID_NAME = 'test_key_1';

export function deriveContext(address: Uint8Array, domain: Uint8Array = DEFAULT_CONTEXT_PREFIX): Uint8Array {
  const context = new Uint8Array(domain.length + address.length);
  context.set(domain, 0);
  context.set(address, domain.length);
  return context;
}

export function textToUint8(text: string): Uint8Array {
  return encoder.encode(text);
}
