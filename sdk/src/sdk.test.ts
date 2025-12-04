import type { Agent } from '@dfinity/agent';
import { Principal } from '@dfinity/principal';
import { describe, expect, it } from 'vitest';

import { StealthClientFactory, Zerc20Sdk, type StealthClientConfig } from './sdk.js';

const STORAGE_ID = 'aaaaa-aa';
const KEY_MANAGER_ID = 'bd3sg-teaaa-aaaaa-qaaba-cai';

function agentStub(): Agent {
  return {} as Agent;
}

describe('StealthClientFactory', () => {
  it('applies defaults and normalizes principals', () => {
    const defaults: Partial<StealthClientConfig> = {
      agent: agentStub(),
      storageCanisterId: STORAGE_ID,
    };
    const factory = new StealthClientFactory(defaults);

    const client = factory.create({
      keyManagerCanisterId: Principal.fromText(KEY_MANAGER_ID),
    });

    expect(client.getStorageCanisterId().toText()).toBe(STORAGE_ID);
    expect(client.getKeyManagerCanisterId().toText()).toBe(KEY_MANAGER_ID);
  });

  it('throws if required fields are missing after merging defaults', () => {
    const factory = new StealthClientFactory();
    expect(() => factory.create()).toThrow(/stealth client agent is required/);
  });
});

describe('Zerc20Sdk', () => {
  it('reuses the stealth client defaults supplied at construction', () => {
    const sdk = new Zerc20Sdk({
      stealth: {
        defaults: {
          agent: agentStub(),
          storageCanisterId: STORAGE_ID,
          keyManagerCanisterId: KEY_MANAGER_ID,
        },
      },
    });

    const client = sdk.createStealthClient();
    expect(client.getStorageCanisterId().toText()).toBe(STORAGE_ID);
    expect(client.getKeyManagerCanisterId().toText()).toBe(KEY_MANAGER_ID);
  });
});
