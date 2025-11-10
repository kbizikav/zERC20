import { getBytes, JsonRpcSigner, keccak256 } from 'ethers';

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
    ensureSigner: () => Promise<JsonRpcSigner>,
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
    ensureSigner: () => Promise<JsonRpcSigner>,
  ): Promise<string> {
    const signer = await ensureSigner();
    const message = await getSeedMessage();
    const signature = await signer.signMessage(message);
    const digest = keccak256(getBytes(signature));
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
  const globalRef = typeof globalThis === 'undefined' ? undefined : (globalThis as { localStorage?: Storage });
  return {
    load(account: string): string | undefined {
      const storage = globalRef?.localStorage;
      if (!storage) {
        return undefined;
      }
      try {
        const stored = storage.getItem(`${prefix}${account}`);
        return isHexSeed(stored ?? undefined) ? stored ?? undefined : undefined;
      } catch {
        return undefined;
      }
    },
    save(account: string, seedHex: string): void {
      const storage = globalRef?.localStorage;
      if (!storage) {
        return;
      }
      try {
        storage.setItem(`${prefix}${account}`, seedHex);
      } catch {
        // ignore storage errors
      }
    },
    remove(account: string): void {
      const storage = globalRef?.localStorage;
      if (!storage) {
        return;
      }
      try {
        storage.removeItem(`${prefix}${account}`);
      } catch {
        // ignore storage errors
      }
    },
  };
}
