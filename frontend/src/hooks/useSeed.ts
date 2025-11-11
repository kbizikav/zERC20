import type { JsonRpcSigner } from 'ethers';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { keccak256, toBytes } from 'viem';
import { getSeedMessage, useStorageStore } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';
import { toAccountKey } from '@utils/accountKey';

export interface SeedState {
  seedHex?: string;
  isDeriving: boolean;
  error?: string;
  deriveSeed: () => Promise<string>;
  clearSeed: () => void;
}

const pendingSeedDerivations = new Map<string, Promise<string>>();

const useStorageHydration = (): boolean => {
  const [hydrated, setHydrated] = useState(() => useStorageStore.persist?.hasHydrated?.() ?? false);

  useEffect(() => {
    if (hydrated) {
      return;
    }
    const unsubscribe = useStorageStore.persist?.onFinishHydration?.(() => {
      setHydrated(true);
    });
    return () => {
      unsubscribe?.();
    };
  }, [hydrated]);

  return hydrated;
};

function normalizeSeedError(error: unknown): string {
  if (typeof error === 'string') {
    return error;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return 'Failed to authorize privacy features. Check your wallet and try again.';
}

function loadStoredSeed(account: string): string | undefined {
  return useStorageStore.getState().seeds[account];
}

function persistSeed(account: string, seed: string): void {
  useStorageStore.getState().setSeed(account, seed);
}

function removeStoredSeed(account: string): void {
  useStorageStore.getState().removeSeed(account);
}

async function deriveSeedForAccount(
  account: string,
  ensureSigner: () => Promise<JsonRpcSigner>,
  force: boolean,
): Promise<string> {
  if (!account.startsWith('0x')) {
    throw new Error('Wallet address must be a hex string.');
  }
  if (!force) {
    const cached = loadStoredSeed(account);
    if (cached) {
      return cached;
    }
  }

  const pending = pendingSeedDerivations.get(account);
  if (pending) {
    return pending;
  }

  if (force) {
    removeStoredSeed(account);
  }

  const derivation = (async () => {
    const message = await getSeedMessage();
    const signer = await ensureSigner();
    const signature = await signer.signMessage(message);
    const digest = keccak256(toBytes(signature));
    persistSeed(account, digest);
    return digest;
  })();

  pendingSeedDerivations.set(account, derivation);
  try {
    return await derivation;
  } finally {
    const current = pendingSeedDerivations.get(account);
    if (current === derivation) {
      pendingSeedDerivations.delete(account);
    }
  }
}

export function useSeed(): SeedState {
  const { ensureSigner, account } = useWallet();
  const accountKey = useMemo(() => toAccountKey(account), [account]);
  const storedSeed = useStorageStore((state) => (accountKey ? state.seeds[accountKey] : undefined));
  const storageHydrated = useStorageHydration();
  const [seedHex, setSeedHex] = useState<string>();
  const [error, setError] = useState<string>();
  const [isDeriving, setIsDeriving] = useState(false);
  const accountRef = useRef(accountKey);

  useEffect(() => {
    accountRef.current = accountKey;
  }, [accountKey]);

  const clearSeed = useCallback(() => {
    const currentAccount = accountRef.current;
    setSeedHex(undefined);
    setError(undefined);
    if (currentAccount) {
      removeStoredSeed(currentAccount);
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
      const message = normalizeSeedError(err);
      setError(message);
      throw err;
    } finally {
      setIsDeriving(false);
    }
  }, [ensureSigner]);

  useEffect(() => {
    if (!accountKey) {
      setSeedHex(undefined);
      setError(undefined);
      setIsDeriving(false);
      return;
    }

    if (!storageHydrated) {
      setSeedHex(undefined);
      setError(undefined);
      setIsDeriving(true);
      return;
    }

    if (storedSeed) {
      setSeedHex(storedSeed);
      setError(undefined);
      setIsDeriving(false);
      return;
    }

    setSeedHex(undefined);
    setError(undefined);

    let cancelled = false;
    setIsDeriving(true);
    setError(undefined);
    deriveSeedForAccount(accountKey, ensureSigner, false)
      .then((digest) => {
        if (!cancelled && accountRef.current === accountKey) {
          setSeedHex(digest);
        }
      })
      .catch((err) => {
        if (!cancelled && accountRef.current === accountKey) {
          const message = normalizeSeedError(err);
          setError(message);
        }
      })
      .finally(() => {
        if (!cancelled && accountRef.current === accountKey) {
          setIsDeriving(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [accountKey, ensureSigner, storageHydrated, storedSeed]);

  return {
    seedHex,
    isDeriving,
    error,
    deriveSeed,
    clearSeed,
  };
}
