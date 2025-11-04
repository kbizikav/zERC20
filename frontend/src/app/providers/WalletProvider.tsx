import {
  PropsWithChildren,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { BrowserProvider, JsonRpcSigner } from 'ethers';

export class WalletProviderError extends Error {
  constructor(message: string, public readonly cause?: unknown) {
    super(message);
    this.name = 'WalletProviderError';
  }
}

export interface SwitchChainOptions {
  chainName?: string;
  rpcUrls?: string[];
  blockExplorerUrls?: string[];
  nativeCurrency?: {
    name: string;
    symbol: string;
    decimals: number;
  };
}

export interface WalletContextValue {
  account?: string;
  chainId?: number;
  provider?: BrowserProvider;
  signer?: JsonRpcSigner;
  status: 'idle' | 'connecting' | 'connected' | 'error';
  error?: WalletProviderError;
  connect: () => Promise<JsonRpcSigner>;
  ensureSigner: () => Promise<JsonRpcSigner>;
  switchChain: (targetChainId: bigint | number, options?: SwitchChainOptions) => Promise<void>;
  resetError: () => void;
  disconnect: () => void;
}

const WalletContext = createContext<WalletContextValue | null>(null);

const listenersInitialState = {
  accountsChanged: null as ((accounts: string[]) => void) | null,
  chainChanged: null as ((chainId: string | number) => void) | null,
} as const;

export function WalletProvider({ children }: PropsWithChildren): JSX.Element {
  const [account, setAccount] = useState<string>();
  const [chainId, setChainId] = useState<number>();
  const [provider, setProvider] = useState<BrowserProvider>();
  const [signer, setSigner] = useState<JsonRpcSigner>();
  const [status, setStatus] = useState<WalletContextValue['status']>('idle');
  const [error, setError] = useState<WalletProviderError>();

  const listeners = useRef({ ...listenersInitialState });
  const abortRef = useRef<AbortController | null>(null);

  const resetListeners = useCallback(() => {
    const { current } = listeners;
    if (!current) {
      return;
    }
    if (current.accountsChanged && window.ethereum?.removeListener) {
      window.ethereum.removeListener('accountsChanged', current.accountsChanged);
    }
    if (current.chainChanged && window.ethereum?.removeListener) {
      window.ethereum.removeListener('chainChanged', current.chainChanged);
    }
    listeners.current = { ...listenersInitialState };
  }, []);

  useEffect(() => resetListeners, [resetListeners]);

  const disconnect = useCallback(() => {
    resetListeners();
    abortRef.current?.abort();
    abortRef.current = null;
    setAccount(undefined);
    setChainId(undefined);
    setProvider(undefined);
    setSigner(undefined);
    setStatus('idle');
    setError(undefined);
  }, [resetListeners]);

  const connect = useCallback(async (): Promise<JsonRpcSigner> => {
    if (!window.ethereum) {
      const err = new WalletProviderError(
        'No injected wallet found. Install MetaMask or another EIP-1193 compatible provider.',
      );
      setError(err);
      setStatus('error');
      throw err;
    }

    setStatus('connecting');
    setError(undefined);
    abortRef.current?.abort();
    const abortController = new AbortController();
    abortRef.current = abortController;

    try {
      const browserProvider = new BrowserProvider(window.ethereum as any, 'any');
      const accounts = (await browserProvider.send('eth_requestAccounts', [])) as string[];
      if (!accounts || accounts.length === 0) {
        throw new WalletProviderError('Wallet did not return any accounts.');
      }

      const nextSigner = await browserProvider.getSigner();
      const network = await browserProvider.getNetwork();

      if (abortController.signal.aborted) {
        throw new WalletProviderError('Wallet connection was cancelled.');
      }

      const lowerAccount = accounts[0]?.toLowerCase();
      setProvider(browserProvider);
      setSigner(nextSigner);
      setAccount(lowerAccount);
      setChainId(Number(network.chainId));
      setStatus('connected');

      resetListeners();
      if (window.ethereum?.on) {
        const handleAccountsChanged = (nextAccounts: string[]) => {
          const accountValue = nextAccounts?.[0]?.toLowerCase();
          if (!accountValue) {
            disconnect();
            setStatus('idle');
            return;
          }
          void (async () => {
            try {
              const refreshedSigner = await browserProvider.getSigner();
              const refreshedNetwork = await browserProvider.getNetwork();
              setSigner(refreshedSigner);
              setAccount(accountValue);
              setChainId(Number(refreshedNetwork.chainId));
            } catch (cause) {
              setError(new WalletProviderError('Failed to refresh signer.', cause));
            }
          })();
        };

        const handleChainChanged = (nextChain: string | number) => {
          const numericChain =
            typeof nextChain === 'string' ? Number.parseInt(nextChain, 16) || Number(nextChain) : Number(nextChain);
          if (Number.isNaN(numericChain)) {
            setChainId(undefined);
            return;
          }
          setChainId(numericChain);
        };

        window.ethereum.on('accountsChanged', handleAccountsChanged);
        window.ethereum.on('chainChanged', handleChainChanged);
        listeners.current = {
          accountsChanged: handleAccountsChanged,
          chainChanged: handleChainChanged,
        };
      }

      return nextSigner;
    } catch (cause) {
      disconnect();
      const err =
        cause instanceof WalletProviderError
          ? cause
          : new WalletProviderError('Failed to connect wallet.', cause);
      setError(err);
      setStatus('error');
      throw err;
    }
  }, [disconnect, resetListeners]);

  const ensureSigner = useCallback(async () => {
    if (signer) {
      return signer;
    }
    return connect();
  }, [connect, signer]);

  const switchChain = useCallback(
    async (targetChainId: bigint | number, options?: SwitchChainOptions) => {
      if (!window.ethereum) {
        throw new WalletProviderError(
          'No injected wallet found. Install MetaMask or another EIP-1193 compatible provider.',
        );
      }

      try {
        const normalized = typeof targetChainId === 'bigint' ? targetChainId : BigInt(targetChainId);
        if (!Number.isSafeInteger(Number(normalized))) {
          throw new WalletProviderError('Chain id is out of supported range.');
        }

        if (chainId !== undefined && BigInt(chainId) === normalized) {
          return;
        }

        const hexChainId = `0x${normalized.toString(16)}`;
        await (window.ethereum as any).request({
          method: 'wallet_switchEthereumChain',
          params: [{ chainId: hexChainId }],
        });

        setChainId(Number(normalized));
        if (provider) {
          try {
            const refreshedSigner = await provider.getSigner();
            setSigner(refreshedSigner);
          } catch (cause) {
            setError(new WalletProviderError('Failed to refresh signer after chain switch.', cause));
          }
        }
      } catch (cause) {
        if (cause instanceof WalletProviderError) {
          setError(cause);
          throw cause;
        }

        const extractCode = (value: unknown): number | undefined => {
          if (!value || typeof value !== 'object') {
            return undefined;
          }
          const codeCandidate = (value as { code?: number }).code;
          if (typeof codeCandidate === 'number') {
            return codeCandidate;
          }
          return extractCode((value as { data?: unknown }).data);
        };

        const errorCode = extractCode(cause);
        const normalized = typeof targetChainId === 'bigint' ? targetChainId : BigInt(targetChainId);
        const hexChainId = `0x${normalized.toString(16)}`;

        if (errorCode === 4902) {
          const rpcUrls = options?.rpcUrls ?? [];
          if (rpcUrls.length === 0) {
            const errorInstance = new WalletProviderError(
              'Selected network is not available in your wallet and no RPC URLs were provided to add it automatically.',
              cause,
            );
            setError(errorInstance);
            throw errorInstance;
          }

          const addParams: Record<string, unknown> = {
            chainId: hexChainId,
            chainName: options?.chainName ?? `Chain ${normalized}`,
            rpcUrls,
            nativeCurrency: options?.nativeCurrency ?? {
              name: 'Ether',
              symbol: 'ETH',
              decimals: 18,
            },
          };

          if (options?.blockExplorerUrls?.length) {
            addParams.blockExplorerUrls = options.blockExplorerUrls;
          }

          try {
            await (window.ethereum as any).request({
              method: 'wallet_addEthereumChain',
              params: [addParams],
            });

            await (window.ethereum as any).request({
              method: 'wallet_switchEthereumChain',
              params: [{ chainId: hexChainId }],
            });

            setChainId(Number(normalized));
            if (provider) {
              try {
                const refreshedSigner = await provider.getSigner();
                setSigner(refreshedSigner);
              } catch (refreshCause) {
                setError(new WalletProviderError('Failed to refresh signer after chain switch.', refreshCause));
              }
            }
            return;
          } catch (addCause) {
            const errorInstance = new WalletProviderError(
              'Failed to add the selected network to your wallet.',
              addCause,
            );
            setError(errorInstance);
            throw errorInstance;
          }
        }

        const errorInstance = new WalletProviderError('Failed to switch wallet network.', cause);
        setError(errorInstance);
        throw errorInstance;
      }
    },
    [chainId, provider],
  );

  const resetError = useCallback(() => setError(undefined), []);

  const value = useMemo<WalletContextValue>(
    () => ({
      account,
      chainId,
      provider,
      signer,
      status,
      error,
      connect,
      ensureSigner,
      switchChain,
      resetError,
      disconnect,
    }),
    [
      account,
      chainId,
      provider,
      signer,
      status,
      error,
      connect,
      ensureSigner,
      switchChain,
      resetError,
      disconnect,
    ],
  );

  return <WalletContext.Provider value={value}>{children}</WalletContext.Provider>;
}

export function useWallet(): WalletContextValue {
  const ctx = useContext(WalletContext);
  if (!ctx) {
    throw new Error('useWallet must be used within WalletProvider');
  }
  return ctx;
}
