import { useCallback, useEffect, useRef, useState } from 'react';
import { getBytes, JsonRpcSigner, keccak256 } from 'ethers';
import { getSeedMessage } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';

export interface SeedState {
  seedHex?: string;
  isDeriving: boolean;
  error?: string;
  deriveSeed: () => Promise<string>;
  clearSeed: () => void;
}

const SEED_STORAGE_PREFIX = 'zerc20:seed:';

const memorySeedCache = new Map<string, string>();
const pendingSeedDerivations = new Map<string, Promise<string>>();

function seedStorageKey(account: string): string {
  return `${SEED_STORAGE_PREFIX}${account}`;
}

function loadStoredSeed(account: string): string | undefined {
  if (memorySeedCache.has(account)) {
    return memorySeedCache.get(account);
  }
  if (typeof window === 'undefined') {
    return undefined;
  }
  try {
    const stored = window.localStorage.getItem(seedStorageKey(account));
    if (stored && /^0x[0-9a-fA-F]+$/.test(stored)) {
      memorySeedCache.set(account, stored);
      return stored;
    }
  } catch {
    // ignore storage errors
  }
  return undefined;
}

function persistSeed(account: string, seed: string): void {
  memorySeedCache.set(account, seed);
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(seedStorageKey(account), seed);
  } catch {
    // ignore storage write failures
  }
}

function forgetSeed(account: string): void {
  memorySeedCache.delete(account);
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(seedStorageKey(account));
  } catch {
    // ignore storage errors
  }
}

async function deriveSeedForAccount(
  account: string,
  ensureSigner: () => Promise<JsonRpcSigner>,
  force: boolean,
): Promise<string> {
  if (!force) {
    const cached = loadStoredSeed(account);
    if (cached) {
      return cached;
    }
    const pending = pendingSeedDerivations.get(account);
    if (pending) {
      return pending;
    }
  } else {
    forgetSeed(account);
  }

  const derivation = (async () => {
    const signer = await ensureSigner();
    const message = await getSeedMessage();
    const signature = await signer.signMessage(message);
    const digest = keccak256(getBytes(signature));
    persistSeed(account, digest);
    return digest;
  })();

  pendingSeedDerivations.set(account, derivation);
  try {
    const result = await derivation;
    return result;
  } finally {
    const currentPending = pendingSeedDerivations.get(account);
    if (currentPending === derivation) {
      pendingSeedDerivations.delete(account);
    }
  }
}

export function useSeed(): SeedState {
  const { ensureSigner, account } = useWallet();
  const [seedHex, setSeedHex] = useState<string>();
  const [error, setError] = useState<string>();
  const [isDeriving, setIsDeriving] = useState(false);
  const accountRef = useRef(account);

  const clearSeed = useCallback(() => {
    const currentAccount = accountRef.current;
    setSeedHex(undefined);
    setError(undefined);
    if (currentAccount) {
      forgetSeed(currentAccount);
    }
  }, []);

  const deriveSeed = useCallback(async () => {
    const currentAccount = accountRef.current;
    setIsDeriving(true);
    setError(undefined);
    try {
      if (!currentAccount) {
        throw new Error('Connect your wallet to derive the privacy seed.');
      }
      const digest = await deriveSeedForAccount(currentAccount, ensureSigner, true);
      if (accountRef.current === currentAccount) {
        setSeedHex(digest);
      }
      return digest;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      throw err;
    } finally {
      setIsDeriving(false);
    }
  }, [ensureSigner]);

  useEffect(() => {
    accountRef.current = account;
    if (!account) {
      setSeedHex(undefined);
      setError(undefined);
      setIsDeriving(false);
      return;
    }

    setSeedHex(undefined);
    setError(undefined);

    const cached = loadStoredSeed(account);
    if (cached) {
      setSeedHex(cached);
      setError(undefined);
      setIsDeriving(false);
      return;
    }

    let cancelled = false;
    setIsDeriving(true);
    setError(undefined);

    deriveSeedForAccount(account, ensureSigner, false)
      .then((digest) => {
        if (!cancelled && accountRef.current === account) {
          setSeedHex(digest);
        }
      })
      .catch((err) => {
        if (!cancelled && accountRef.current === account) {
          const message = err instanceof Error ? err.message : String(err);
          setError(message);
        }
      })
      .finally(() => {
        if (!cancelled && accountRef.current === account) {
          setIsDeriving(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [account, ensureSigner]);

  return {
    seedHex,
    isDeriving,
    error,
    deriveSeed,
    clearSeed,
  };
}
