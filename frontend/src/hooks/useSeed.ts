import { useCallback, useEffect, useRef, useState } from 'react';
import { SeedManager, createBrowserSeedStorage } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';

export interface SeedState {
  seedHex?: string;
  isDeriving: boolean;
  error?: string;
  deriveSeed: () => Promise<string>;
  clearSeed: () => void;
}

const seedManager = new SeedManager({
  storage: createBrowserSeedStorage(),
});

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
      void seedManager.clearSeed(currentAccount);
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
      const digest = await seedManager.deriveSeed(currentAccount, ensureSigner, { force: true });
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

    let cancelled = false;
    setIsDeriving(true);
    setError(undefined);
    const cached = seedManager.getCachedSeed(account);
    if (cached) {
      setSeedHex(cached);
      setIsDeriving(false);
      return;
    }

    seedManager
      .deriveSeed(account, ensureSigner, { force: false })
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
