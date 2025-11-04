import { beforeAll, describe, expect, test } from 'vitest';
import { HttpAgent, AnonymousIdentity } from '@dfinity/agent';
import { Principal } from '@dfinity/principal';
import { webcrypto } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { StealthCanisterClient } from '../client.js';
import { AnnouncementInput } from '../types.js';
import { StealthError } from '../errors.js';

type LocalnetConfig = {
  host: string;
  storageCanisterId: string;
  keyManagerCanisterId: string;
};

type CanisterIdsFile = Record<string, Record<string, string>>;

const localnetConfig = (() => {
  try {
    return loadLocalnetConfig();
  } catch (error) {
    if (error instanceof StealthError) {
      return null;
    }
    throw error;
  }
})();

let client: StealthCanisterClient;

if (!localnetConfig) {
  describe.skip('announce storage canister (localnet)', () => {
    test('submitAnnouncement stores and listAnnouncements/getAnnouncement return the same entry', () => {
      // skipped when localnet configuration is unavailable
    });
  });
} else {
  beforeAll(async () => {
    if (typeof globalThis.crypto === 'undefined') {
      (globalThis as unknown as { crypto: Crypto }).crypto = webcrypto as unknown as Crypto;
    }

    const agent = new HttpAgent({
      host: localnetConfig.host,
      identity: new AnonymousIdentity(),
    });

    const url = new URL(localnetConfig.host);
    if (['127.0.0.1', 'localhost'].includes(url.hostname)) {
      await agent.fetchRootKey();
    }

    client = new StealthCanisterClient(
      agent,
      Principal.fromText(localnetConfig.storageCanisterId),
      Principal.fromText(localnetConfig.keyManagerCanisterId),
    );
  });

  describe('announce storage canister (localnet)', () => {
    test('submitAnnouncement stores and listAnnouncements/getAnnouncement return the same entry', async () => {
      const input = randomAnnouncement();

      const created = await client.submitAnnouncement(input);
      expect(created.id).toBeTypeOf('bigint');
      expect(created.ciphertext).toEqual(input.ciphertext);
      expect(created.ibeCiphertext).toEqual(input.ibeCiphertext);
      expect(created.nonce).toEqual(input.nonce);
      expect(created.createdAtNs).toBeTypeOf('bigint');

      const fetched = await client.getAnnouncement(created.id);
      expect(fetched).not.toBeNull();
      expect(fetched?.ciphertext).toEqual(input.ciphertext);
      expect(fetched?.ibeCiphertext).toEqual(input.ibeCiphertext);
      expect(fetched?.nonce).toEqual(input.nonce);

      const page = await client.listAnnouncements();
      const match = page.announcements.find((item) => item.id === created.id);
      expect(match).toBeDefined();
      expect(match?.ciphertext).toEqual(input.ciphertext);
      expect(match?.ibeCiphertext).toEqual(input.ibeCiphertext);
      expect(match?.nonce).toEqual(input.nonce);
    });
  });
}

function randomAnnouncement(): AnnouncementInput {
  return {
    ibeCiphertext: randomBytes(256),
    ciphertext: randomBytes(64),
    nonce: randomBytes(12),
  };
}

function randomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  globalThis.crypto.getRandomValues(bytes);
  return bytes;
}

function loadLocalnetConfig(): LocalnetConfig {
  const host = process.env.IC_HOST ?? process.env.DFX_NETWORK_HOST ?? 'http://127.0.0.1:4943';
  const storageCanisterId =
    getFirstDefined(process.env.CANISTER_ID_STORAGE, process.env.STORAGE_CANISTER_ID) ??
    lookupCanisterIdFromFile('storage');
  const keyManagerCanisterId =
    getFirstDefined(process.env.CANISTER_ID_KEY_MANAGER, process.env.KEY_MANAGER_CANISTER_ID) ??
    lookupCanisterIdFromFile('key_manager');

  if (!storageCanisterId || !keyManagerCanisterId) {
    throw new StealthError(
      'Missing canister IDs for localnet tests. Set CANISTER_ID_STORAGE and CANISTER_ID_KEY_MANAGER or run `dfx deploy` first.',
    );
  }

  return {
    host,
    storageCanisterId,
    keyManagerCanisterId,
  };
}

function getFirstDefined<T>(...values: (T | undefined)[]): T | undefined {
  for (const value of values) {
    if (value !== undefined && value !== null) {
      return value;
    }
  }
  return undefined;
}

function lookupCanisterIdFromFile(canisterName: string): string | undefined {
  const file = readCanisterIdsFile();
  if (!file) {
    return undefined;
  }
  const entry = file[canisterName];
  if (!entry) {
    return undefined;
  }
  return entry.local ?? entry.localnet ?? entry['local-network'];
}

function readCanisterIdsFile(): CanisterIdsFile | undefined {
  const filePath = resolveWorkspacePath('.dfx', 'local', 'canister_ids.json');
  if (!existsSync(filePath)) {
    return undefined;
  }

  try {
    const contents = readFileSync(filePath, 'utf8');
    return JSON.parse(contents) as CanisterIdsFile;
  } catch (error) {
    throw new StealthError(`Failed to read canister ids from ${filePath}: ${(error as Error).message}`);
  }
}

function resolveWorkspacePath(...segments: string[]): string {
  const testDir = path.dirname(fileURLToPath(import.meta.url));
  return path.resolve(testDir, '../../../..', ...segments);
}
