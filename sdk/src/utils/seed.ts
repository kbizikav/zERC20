import type { WalletClient } from 'viem';
import { keccak256 } from 'viem';
import { type StateStorage } from 'zustand/middleware';

import { normalizeHex } from './hex.js';
import { getSeedMessage } from '../wasm/index.js';

export interface SeedStorage {
  load(account: string): string | undefined;
  save(account: string, seedHex: string): void;
  remove(account: string): void;
}

export interface SeedManagerOptions {
  storage?: SeedStorage;
}

function isHexSeed(seed: string | undefined): seed is string {
  return typeof seed === 'string' && /^0x[0-9a-fA-F]+$/.test(seed);
}

type LegacySigner = {
  signMessage(message: string | Uint8Array): Promise<string>;
};

type SeedSigner = LegacySigner | WalletClient;

function isWalletClient(value: unknown): value is WalletClient {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as WalletClient).signMessage === 'function' &&
    typeof (value as WalletClient).request === 'function'
  );
}

async function signMessageWithSigner(
  signer: SeedSigner,
  account: string,
  message: string,
): Promise<string> {
  if (isWalletClient(signer)) {
    const normalizedAccount = normalizeHex(account) as `0x${string}`;
    return signer.signMessage({ account: normalizedAccount, message });
  }
  return signer.signMessage(message);
}

export class SeedManager {
  private readonly storage?: SeedStorage;
  private readonly cache = new Map<string, string>();
  private readonly pending = new Map<string, Promise<string>>();

  constructor(options: SeedManagerOptions = {}) {
    this.storage = options.storage;
  }

  getCachedSeed(account: string): string | undefined {
    return this.cache.get(account);
  }

  async deriveSeed(
    account: string,
    ensureSigner: () => Promise<SeedSigner>,
    options: { force?: boolean } = {},
  ): Promise<string> {
    if (!account) {
      throw new Error('account is required to derive seed');
    }
    const force = options.force ?? false;
    if (!force) {
      const existing = this.cache.get(account) ?? this.loadFromStorage(account);
      if (existing) {
        this.cache.set(account, existing);
        return existing;
      }
      const pending = this.pending.get(account);
      if (pending) {
        return pending;
      }
    } else {
      await this.clearSeed(account);
    }

    const derivation = this.performDerivation(account, ensureSigner);
    this.pending.set(account, derivation);
    try {
      const seed = await derivation;
      return seed;
    } finally {
      const current = this.pending.get(account);
      if (current === derivation) {
        this.pending.delete(account);
      }
    }
  }

  async clearSeed(account: string): Promise<void> {
    this.cache.delete(account);
    if (this.storage) {
      try {
        this.storage.remove(account);
      } catch {
        // ignore storage errors
      }
    }
  }

  private loadFromStorage(account: string): string | undefined {
    if (!this.storage) {
      return undefined;
    }
    try {
      const stored = this.storage.load(account);
      if (isHexSeed(stored)) {
        this.cache.set(account, stored);
        return stored;
      }
    } catch {
      // ignore storage errors
    }
    return undefined;
  }

  private async performDerivation(
    account: string,
    ensureSigner: () => Promise<SeedSigner>,
  ): Promise<string> {
    const signer = await ensureSigner();
    const message = await getSeedMessage();
    const signature = await signMessageWithSigner(signer, account, message);
    const normalizedSignature = normalizeHex(signature) as `0x${string}`;
    const digest = normalizeHex(keccak256(normalizedSignature));
    this.cache.set(account, digest);
    if (this.storage) {
      try {
        this.storage.save(account, digest);
      } catch {
        // ignore storage errors
      }
    }
    return digest;
  }
}

export function createBrowserSeedStorage(prefix = 'zerc20:seed:'): SeedStorage {
  const stateStorage = resolveStateStorage();
  return {
    load(account: string): string | undefined {
      try {
        const stored = readStorageValue(stateStorage, `${prefix}${account}`);
        return isHexSeed(stored) ? stored : undefined;
      } catch {
        return undefined;
      }
    },
    save(account: string, seedHex: string): void {
      try {
        void stateStorage.setItem(`${prefix}${account}`, seedHex);
      } catch {
        // ignore storage errors
      }
    },
    remove(account: string): void {
      try {
        void stateStorage.removeItem(`${prefix}${account}`);
      } catch {
        // ignore storage errors
      }
    },
  };
}

const noopStorage: StateStorage = {
  getItem: () => null,
  setItem: () => undefined,
  removeItem: () => undefined,
};

function resolveStateStorage(): StateStorage {
  if (typeof globalThis === 'undefined') {
    return noopStorage;
  }
  const storage = (globalThis as { localStorage?: StateStorage }).localStorage;
  return storage ?? noopStorage;
}

function readStorageValue(storage: StateStorage, key: string): string | undefined {
  const value = storage.getItem(key);
  if (isPromiseLike(value)) {
    return undefined;
  }
  return (value as string | null) ?? undefined;
}

function isPromiseLike<T>(value: unknown): value is PromiseLike<T> {
  return typeof value === 'object' && value !== null && typeof (value as PromiseLike<T>).then === 'function';
}
