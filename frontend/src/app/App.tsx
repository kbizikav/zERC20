import { ChangeEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { formatUnits } from 'ethers';
import { AppProviders } from './providers/AppProviders';
import { useWallet } from './providers/WalletProvider';
import { Tabs } from '@components/Tabs';
import { useRuntimeConfig } from '@config/ConfigContext';
import type { NormalizedTokens, TeleportArtifacts } from '@/types/app';
import { loadTeleportArtifacts, loadTokens } from '@services/resources';
import { getZerc20Contract } from '@services/sdk';
import { ConvertPanel, PrivateReceivePanel, PrivateSendPanel, ScanReceivingsPanel } from '@features/index';
import { configureWasmLocator } from '@services/sdk/wasm/index.js';
import { buildSwitchChainOptions } from '@/utils/wallet';

const TAB_SEND = 'send';
const TAB_RECEIVE = 'receive';
const TAB_RECEIVINGS = 'receivings';

type ActiveTab = typeof TAB_SEND | typeof TAB_RECEIVE | typeof TAB_RECEIVINGS;

function compactBalanceDisplay(value: bigint, decimals: number): string {
  const normalizedDecimals = Number.isFinite(decimals) && decimals >= 0 ? Math.min(decimals, 18) : 18;
  const formatted = formatUnits(value, normalizedDecimals);
  const [whole, fractional = ''] = formatted.split('.');
  if (!fractional) {
    return whole;
  }
  const trimmedFractional = fractional.slice(0, 4).replace(/0+$/, '');
  return trimmedFractional ? `${whole}.${trimmedFractional}` : whole;
}

function ConnectionCard({
  tokens,
  isReady,
  loadingMessage,
  error,
  onClearStorage,
  onShowConvert,
  convertDisabled,
  tokenSymbol,
}: {
  tokens?: NormalizedTokens | null;
  loadingMessage?: string;
  error?: string;
  isReady: boolean;
  onClearStorage?: () => boolean;
  onShowConvert?: () => void;
  convertDisabled?: boolean;
  tokenSymbol: string;
}): JSX.Element {
  const wallet = useWallet();
  const [formattedBalance, setFormattedBalance] = useState<string>();
  const [balanceLoading, setBalanceLoading] = useState(false);
  const [balanceError, setBalanceError] = useState<string>();
  const [storageStatus, setStorageStatus] = useState<string>();
  const [selectedChainId, setSelectedChainId] = useState<bigint>();
  const [chainSwitching, setChainSwitching] = useState(false);
  const [chainError, setChainError] = useState<string>();

  const availableTokens = tokens?.tokens ?? [];
  const hasAvailableTokens = availableTokens.length > 0;
  const isWalletConnected = wallet.status === 'connected';
  const walletChainIdBigInt =
    wallet.chainId !== undefined && wallet.chainId !== null ? BigInt(wallet.chainId) : undefined;

  useEffect(() => {
    const tokenList = tokens?.tokens ?? [];
    if (!tokenList.length) {
      setSelectedChainId(undefined);
      return;
    }

    if (walletChainIdBigInt && tokenList.some((token) => token.chainId === walletChainIdBigInt)) {
      setSelectedChainId((prev) => (prev === walletChainIdBigInt ? prev : walletChainIdBigInt));
      return;
    }

    setSelectedChainId((prev) => {
      if (prev && tokenList.some((token) => token.chainId === prev)) {
        return prev;
      }
      return tokenList[0]?.chainId;
    });
  }, [tokens?.tokens, walletChainIdBigInt]);

  useEffect(() => {
    setChainError(undefined);
  }, [wallet.chainId]);

  const selectedToken = useMemo(
    () => tokens?.tokens?.find((entry) => entry.chainId === selectedChainId),
    [tokens?.tokens, selectedChainId],
  );

  const connectedToken = useMemo(
    () =>
      walletChainIdBigInt
        ? tokens?.tokens?.find((entry) => entry.chainId === walletChainIdBigInt)
        : undefined,
    [tokens?.tokens, walletChainIdBigInt],
  );

  const isChainMatch =
    walletChainIdBigInt !== undefined &&
    selectedChainId !== undefined &&
    walletChainIdBigInt === selectedChainId;

  const selectedChainLabel = useMemo(() => {
    if (!selectedToken) {
      return hasAvailableTokens ? 'Select a network' : 'No networks available';
    }
    return `${selectedToken.label} (#${selectedToken.chainId.toString()})`;
  }, [hasAvailableTokens, selectedToken]);

  const walletChainLabel = useMemo(() => {
    if (connectedToken) {
      return `${connectedToken.label} (#${connectedToken.chainId.toString()})`;
    }
    if (wallet.chainId !== undefined && wallet.chainId !== null) {
      return `#${wallet.chainId}`;
    }
    return undefined;
  }, [connectedToken, wallet.chainId]);

  const walletAddress = wallet.account?.trim() || undefined;
  const connectionChainText = walletChainLabel ?? selectedChainLabel ?? 'Not available';

  const handleChainSelect = useCallback(
    async (chainId: bigint) => {
      setSelectedChainId(chainId);
      setChainError(undefined);

      if (!isWalletConnected || !wallet.switchChain) {
        return;
      }

      const current = walletChainIdBigInt;
      if (current && current === chainId) {
        return;
      }

      setChainSwitching(true);
      try {
        const matchedToken = tokens?.tokens?.find((entry) => entry.chainId === chainId);
        const switchOptions = matchedToken ? buildSwitchChainOptions(matchedToken) : undefined;
        await wallet.switchChain(chainId, switchOptions);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setChainError(message);
        if (current) {
          setSelectedChainId(current);
        }
      } finally {
        setChainSwitching(false);
      }
    },
    [isWalletConnected, tokens?.tokens, wallet, walletChainIdBigInt],
  );

  const handleChainSelectChange = useCallback(
    (event: ChangeEvent<HTMLSelectElement>) => {
      const value = event.target.value;
      if (!value) {
        setSelectedChainId(undefined);
        return;
      }
      try {
        const nextChainId = BigInt(value);
        void handleChainSelect(nextChainId);
      } catch {
        setChainError('Invalid network selected.');
      }
    },
    [handleChainSelect],
  );

  const shouldShowStatusMessages =
    Boolean(wallet.error) || Boolean(loadingMessage) || Boolean(error) || (!isReady && !loadingMessage);

  const activeToken = useMemo(() => {
    if (!tokens?.tokens || wallet.chainId === undefined || wallet.chainId === null) {
      return undefined;
    }
    try {
      const chainAsBigInt = BigInt(wallet.chainId);
      return tokens.tokens.find((entry) => entry.chainId === chainAsBigInt);
    } catch {
      return undefined;
    }
  }, [tokens?.tokens, wallet.chainId]);

  useEffect(() => {
    let cancelled = false;
    const account = wallet.account;
    const provider = wallet.provider;
    const token = activeToken;

    if (!account || !provider || !token) {
      setFormattedBalance(undefined);
      setBalanceError(undefined);
      setBalanceLoading(false);
      return;
    }

    const fetchBalance = async () => {
      setBalanceLoading(true);
      setBalanceError(undefined);
      try {
        const contract = getZerc20Contract(token.tokenAddress, provider);
        const [rawBalance, decimalsValue] = await Promise.all([
          contract.balanceOf(account),
          contract.decimals(),
        ]);
        if (cancelled) {
          return;
        }
        const decimalsNumber = Number(decimalsValue);
        setFormattedBalance(
          compactBalanceDisplay(rawBalance, Number.isFinite(decimalsNumber) ? decimalsNumber : 18),
        );
      } catch (err) {
        if (cancelled) {
          return;
        }
        const message = err instanceof Error ? err.message : String(err);
        setBalanceError(`Failed to load balance: ${message}`);
        setFormattedBalance(undefined);
      } finally {
        if (!cancelled) {
          setBalanceLoading(false);
        }
      }
    };

    void fetchBalance();

    return () => {
      cancelled = true;
    };
  }, [wallet.account, wallet.provider, activeToken]);

  const handleClearStorage = useCallback(() => {
    if (!onClearStorage) {
      return;
    }
    const success = onClearStorage();
    setStorageStatus(success ? 'Local storage cleared.' : 'Failed to clear local storage.');
  }, [onClearStorage]);

  const handleConvertClick = useCallback(() => {
    if (!onShowConvert || convertDisabled) {
      return;
    }
    onShowConvert();
  }, [convertDisabled, onShowConvert]);

  const convertButtonLabel = tokenSymbol ? `Convert ${tokenSymbol}` : 'Convert';

  return (
    <section className="card connection-card">
      <div className="connection-card-top">
        <div className="connection-balance-area">
          <div className="connection-balance-display">
            <span className="balance-label">{tokenSymbol} Balance</span>
            <span className="balance-value">
              {wallet.account && activeToken
                ? balanceLoading
                  ? 'Loading…'
                  : formattedBalance ?? '----'
                : '----'}
            </span>
          </div>
          <button
            type="button"
            className="connection-convert outline"
            onClick={handleConvertClick}
            disabled={convertDisabled || !onShowConvert}
          >
            {convertButtonLabel}
          </button>
        </div>
        <button
          type="button"
          className="connection-action"
          onClick={() => {
            if (wallet.status === 'connected') {
              wallet.disconnect();
              return;
            }
            wallet.connect().catch(() => undefined);
          }}
          disabled={wallet.status === 'connecting'}
        >
          {wallet.status === 'connected' ? 'Disconnect' : 'Connect Wallet'}
        </button>
      </div>

      <div className="connection-divider" role="presentation" />

      <div className="connection-details">
        <div className="connection-detail">
          <span className="connection-detail-label">Connected as</span>
          <div className="connection-detail-value" title={walletAddress ?? undefined}>
            <span className="connection-detail-text">{walletAddress ?? 'Not connected'}</span>
          </div>
        </div>

        <div className="connection-detail">
          <span className="connection-detail-label">Connected Chain</span>
          <div className="connection-detail-value connection-detail-select">
            <span className="connection-detail-text">{connectionChainText}</span>
            {hasAvailableTokens ? (
              <>
                <span className="connection-detail-action" aria-hidden="true">🔄 Change</span>
                <select
                  id="connection-chain-select"
                  aria-label="Select Network"
                  value={selectedChainId !== undefined ? selectedChainId.toString() : ''}
                  onChange={handleChainSelectChange}
                  disabled={chainSwitching}
                  className="connection-select-native"
                >
                  <option value="" disabled={!hasAvailableTokens}>
                    {hasAvailableTokens ? 'Choose a network' : 'No networks available'}
                  </option>
                  {availableTokens.map((token) => (
                    <option key={token.chainId.toString()} value={token.chainId.toString()}>
                      {token.label} (#{token.chainId.toString()})
                    </option>
                  ))}
                </select>
              </>
            ) : null}
          </div>
        </div>
      </div>

      {hasAvailableTokens && (
        <div className="connection-messages">
          {chainSwitching && <p className="connection-note">Switching network in wallet…</p>}
          {chainError && <p className="error">{chainError}</p>}
          {!isWalletConnected && <p className="connection-note">Connect your wallet to switch networks automatically.</p>}
          {isWalletConnected &&
            walletChainIdBigInt !== undefined &&
            selectedChainId !== undefined &&
            !isChainMatch &&
            !chainSwitching && (
              <p className="connection-note">
                Wallet is currently on {walletChainLabel ?? `#${wallet.chainId}`}. Approve the switch in MetaMask to use{' '}
                {selectedChainLabel}.
              </p>
            )}
        </div>
      )}

      {wallet.account && !activeToken && tokens?.tokens?.length ? (
        <p className="error connection-error-inline">
          Switch MetaMask to a supported network to view your zERC20 balance.
        </p>
      ) : null}
      {balanceError && <p className="error connection-error-inline">{balanceError}</p>}

      <div className="connection-storage">
        <div className="connection-storage-left">
          <button type="button" className="ghost" onClick={handleClearStorage}>
            Clear local storage
          </button>
          {storageStatus && <p className="connection-note">{storageStatus}</p>}
        </div>
      </div>

      {shouldShowStatusMessages && (
        <div className="connection-status">
          {wallet.error && <p className="error">{wallet.error.message}</p>}
          {loadingMessage && <p className="connection-note">{loadingMessage}</p>}
          {error && <p className="error">{error}</p>}
          {!isReady && !loadingMessage ? <p className="connection-note">Waiting for configuration and assets…</p> : null}
        </div>
      )}
    </section>
  );
}

function AppContent(): JSX.Element {
  const runtime = useRuntimeConfig();
  const { tokenSymbol } = runtime.app;
  const [tokens, setTokens] = useState<NormalizedTokens | null>(null);
  const [artifacts, setArtifacts] = useState<TeleportArtifacts | null>(null);
  const [loadingMessage, setLoadingMessage] = useState<string>('Loading configuration…');
  const [error, setError] = useState<string>();
  const [activeTab, setActiveTab] = useState<ActiveTab>(TAB_SEND);
  const [storageRevision, setStorageRevision] = useState<number>(0);
  const [isConvertOpen, setIsConvertOpen] = useState<boolean>(false);

  useEffect(() => {
    configureWasmLocator();
  }, []);

  useEffect(() => {
    let cancelled = false;
    const loadResources = async () => {
      try {
        setError(undefined);
        setLoadingMessage('Loading token metadata…');
        const loadedTokens = await loadTokens(runtime.resources.tokensCompressed);
        if (cancelled) return;
        setTokens(loadedTokens);

        setLoadingMessage('Loading proving artifacts…');
        const loadedArtifacts = await loadTeleportArtifacts(runtime.resources.artifacts);
        if (cancelled) return;
        setArtifacts(loadedArtifacts);
        setLoadingMessage('');
      } catch (err) {
        if (cancelled) {
          return;
        }
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setLoadingMessage('');
      }
    };

    loadResources().catch((err) => {
      if (!cancelled) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setLoadingMessage('');
      }
    });

    return () => {
      cancelled = true;
    };
  }, [runtime.resources]);

  const isReady = useMemo(
    () => Boolean(tokens && artifacts && !loadingMessage && !error),
    [tokens, artifacts, loadingMessage, error],
  );

  const hasConvertTokens = useMemo(
    () => Boolean(tokens?.tokens?.some((entry) => Boolean(entry.minterAddress))),
    [tokens?.tokens],
  );

  const handleShowConvert = useCallback(() => {
    setIsConvertOpen(true);
  }, []);

  const handleCloseConvert = useCallback(() => {
    setIsConvertOpen(false);
  }, []);

  const tabItems = useMemo(
    () => [
      { id: TAB_SEND, label: 'Private Sending', disabled: !isReady },
      { id: TAB_RECEIVINGS, label: 'Receivings', disabled: !isReady },
      { id: TAB_RECEIVE, label: 'Generate Burn Address', disabled: !isReady },
    ],
    [isReady],
  );

  const handleClearStorage = useCallback(() => {
    if (typeof window === 'undefined') {
      return false;
    }
    try {
      window.localStorage.clear();
      setStorageRevision((prev) => prev + 1);
      return true;
    } catch {
      return false;
    }
  }, []);

  return (
    <div className="layout">
      <header className="page-header">
        <div className="page-title-group">
          <h1>{tokenSymbol} Console</h1>
          <p className="page-subtitle">Transfer &amp; Receive {tokenSymbol} privately.</p>
        </div>
      </header>

      <ConnectionCard
        tokens={tokens}
        loadingMessage={loadingMessage}
        error={error}
        isReady={isReady}
        onClearStorage={handleClearStorage}
        onShowConvert={handleShowConvert}
        convertDisabled={!isReady || !hasConvertTokens || isConvertOpen}
        tokenSymbol={tokenSymbol}
      />

      <Tabs
        activeId={activeTab}
        onSelect={(id) => setActiveTab(id as ActiveTab)}
        items={tabItems}
        ariaLabel="zERC20 console sections"
      />

      <main className="panels">
        {isReady && tokens && artifacts ? (
          <>
            {activeTab === TAB_SEND && (
              <section
                className="panel"
                role="tabpanel"
                id={`panel-${TAB_SEND}`}
                aria-labelledby={`tab-${TAB_SEND}`}
              >
                <PrivateSendPanel config={runtime.app} tokens={tokens} />
              </section>
            )}
            {activeTab === TAB_RECEIVINGS && (
              <section
                className="panel"
                role="tabpanel"
                id={`panel-${TAB_RECEIVINGS}`}
                aria-labelledby={`tab-${TAB_RECEIVINGS}`}
              >
                <ScanReceivingsPanel
                  config={runtime.app}
                  tokens={tokens}
                  artifacts={artifacts}
                  storageRevision={storageRevision}
                />
              </section>
            )}
            {activeTab === TAB_RECEIVE && (
              <section
                className="panel"
                role="tabpanel"
                id={`panel-${TAB_RECEIVE}`}
                aria-labelledby={`tab-${TAB_RECEIVE}`}
              >
                <PrivateReceivePanel
                  config={runtime.app}
                  tokens={tokens}
                  artifacts={artifacts}
                  storageRevision={storageRevision}
                />
              </section>
            )}
          </>
        ) : (
          <section className="card" role="status" aria-live="polite">
            <p>Waiting for configuration and assets…</p>
          </section>
        )}
      </main>
      {isConvertOpen && tokens ? (
        <div className="modal-overlay" role="dialog" aria-modal="true" aria-label="Convert tokens">
          <div className="modal">
            <div className="modal-header">
              <h3>Convert</h3>
              <button type="button" className="ghost" onClick={handleCloseConvert}>
                Close
              </button>
            </div>
            <div className="modal-body">
              <ConvertPanel tokens={tokens} showHeader={false} />
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export default function App(): JSX.Element {
  return (
    <AppProviders>
      <AppContent />
    </AppProviders>
  );
}
