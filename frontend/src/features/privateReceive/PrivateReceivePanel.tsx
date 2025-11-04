import { FormEvent, MouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  NUM_BATCH_INVOICES,
  buildFullBurnAddress,
  deriveInvoiceBatch,
  deriveInvoiceSingle,
  listInvoices,
  normalizeHex,
  prepareInvoiceIssue,
  submitInvoice,
  isSingleInvoiceHex,
  getZerc20Contract,
  createProviderForToken,
} from '@services/sdk';
import { getBytes, parseUnits } from 'ethers';
import type { ContractRunner } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens, TeleportArtifacts } from '@/types/app';
import type { TokenEntry } from '@services/sdk/registry/tokens.js';
import { useWallet } from '@app/providers/WalletProvider';
import { getStealthClient } from '@services/clients';
import { useSeed } from '@/hooks/useSeed';
import { toDataURL } from 'qrcode';
import { ScanInvoicesPanel } from '@features/scanInvoices/ScanInvoicesPanel';

interface PrivateReceivePanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
  artifacts: TeleportArtifacts;
  storageRevision: number;
}

interface InvoiceResult {
  invoiceId: string;
  signatureHex: string;
  burnAddresses: Array<{
    subId: number;
    burnAddress: string;
    secret: string;
    tweak: string;
  }>;
  signatureMessage: string;
  isBatch: boolean;
}

interface BurnAddressSummary {
  subId: number;
  burnAddress: string;
}

interface LatestBurnSummary {
  invoiceId: string;
  isBatch: boolean;
  burnAddresses: BurnAddressSummary[];
}

export function PrivateReceivePanel({ config, tokens, artifacts, storageRevision }: PrivateReceivePanelProps): JSX.Element {
  const wallet = useWallet();
  const seed = useSeed();
  const [isBatch, setIsBatch] = useState(false);
  const [showOptions, setShowOptions] = useState(false);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<InvoiceResult>();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [selectedBurn, setSelectedBurn] = useState<BurnAddressSummary | null>(null);
  const [qrAmount, setQrAmount] = useState('1');
  const [qrDataUrl, setQrDataUrl] = useState<string>();
  const [qrPayload, setQrPayload] = useState<string>();
  const [qrError, setQrError] = useState<string>();
  const qrAmountRef = useRef('1');
  const selectedBurnRef = useRef<string>();
  const [qrTokenDecimals, setQrTokenDecimals] = useState(18);
  const [latestBurnSummary, setLatestBurnSummary] = useState<LatestBurnSummary | null>(null);
  const [latestBurnError, setLatestBurnError] = useState<string>();
  const [isLoadingLatestBurns, setIsLoadingLatestBurns] = useState(false);
  const [invoiceReloadRevision, setInvoiceReloadRevision] = useState(0);
  const [copyNotice, setCopyNotice] = useState<{ left: number; top: number } | null>(null);
  const copyNoticeTimeoutRef = useRef<number | undefined>(undefined);

  const availableTokens = useMemo(() => tokens.tokens ?? [], [tokens.tokens]);
  const connectedToken = useMemo(() => {
    const currentChain = wallet.chainId;
    if (currentChain === undefined || currentChain === null) {
      return undefined;
    }
    try {
      return availableTokens.find((entry) => entry.chainId === BigInt(currentChain));
    } catch {
      return undefined;
    }
  }, [availableTokens, wallet.chainId]);

  const [qrChainId, setQrChainId] = useState<string>(() => {
    const preferred = connectedToken ?? availableTokens[0];
    return preferred ? preferred.chainId.toString() : '';
  });

  const qrSelectedToken = useMemo<TokenEntry | undefined>(
    () => availableTokens.find((entry) => entry.chainId.toString() === qrChainId),
    [availableTokens, qrChainId],
  );

  useEffect(() => {
    if (availableTokens.length === 0) {
      if (qrChainId !== '') {
        setQrChainId('');
      }
      return;
    }
    const hasSelection = availableTokens.some((entry) => entry.chainId.toString() === qrChainId);
    if (hasSelection) {
      return;
    }
    const preferred = connectedToken ?? availableTokens[0];
    if (preferred) {
      setQrChainId(preferred.chainId.toString());
    }
  }, [availableTokens, connectedToken, qrChainId]);

  const isWalletConnected = Boolean(wallet.account && wallet.chainId);
  const isSupportedNetwork = Boolean(connectedToken);

  useEffect(() => {
    if (!wallet.account || !connectedToken || !seed.seedHex || seed.isDeriving || seed.error) {
      setLatestBurnSummary(null);
      setLatestBurnError(undefined);
      setIsLoadingLatestBurns(false);
      return;
    }

    let cancelled = false;
    const ownerAddress = normalizeHex(wallet.account);
    const chainId = connectedToken.chainId;
    const seedHex = seed.seedHex;

    setIsLoadingLatestBurns(true);
    setLatestBurnError(undefined);

    const loadLatestBurns = async () => {
      try {
        const stealthClient = await getStealthClient(config);
        const invoiceIds = await listInvoices(stealthClient, ownerAddress, chainId);
        if (cancelled) {
          return;
        }
        if (invoiceIds.length === 0) {
          setLatestBurnSummary(null);
          return;
        }
        const normalizedIds = invoiceIds
          .map((value) => normalizeHex(value))
          .sort((a, b) => {
            const aValue = BigInt(a);
            const bValue = BigInt(b);
            if (aValue === bValue) {
              return 0;
            }
            return aValue > bValue ? -1 : 1;
          });
        const latestInvoiceId = normalizedIds[0];
        if (latestBurnSummary?.invoiceId === latestInvoiceId) {
          return;
        }
        const invoiceIsSingle = isSingleInvoiceHex(latestInvoiceId);
        const recipient = ownerAddress;
        const burns: BurnAddressSummary[] = [];
        if (invoiceIsSingle) {
          const secretAndTweak = await deriveInvoiceSingle(seedHex, latestInvoiceId, chainId, recipient);
          if (cancelled) {
            return;
          }
          const burn = await buildFullBurnAddress(chainId, recipient, secretAndTweak.secret, secretAndTweak.tweak);
          if (cancelled) {
            return;
          }
          burns.push({ subId: 0, burnAddress: burn.burnAddress });
        } else {
          for (let subId = 0; subId < NUM_BATCH_INVOICES; subId += 1) {
            const secretAndTweak = await deriveInvoiceBatch(seedHex, latestInvoiceId, subId, chainId, recipient);
            if (cancelled) {
              return;
            }
            const burn = await buildFullBurnAddress(
              chainId,
              recipient,
              secretAndTweak.secret,
              secretAndTweak.tweak,
            );
            if (cancelled) {
              return;
            }
            burns.push({ subId, burnAddress: burn.burnAddress });
          }
        }
        if (!cancelled) {
          setLatestBurnSummary({ invoiceId: latestInvoiceId, isBatch: !invoiceIsSingle, burnAddresses: burns });
          setLatestBurnError(undefined);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (!cancelled) {
          setLatestBurnError(message);
          setLatestBurnSummary(null);
        }
      } finally {
        if (!cancelled) {
          setIsLoadingLatestBurns(false);
        }
      }
    };

    void loadLatestBurns();

    return () => {
      cancelled = true;
    };
  }, [
    wallet.account,
    connectedToken,
    seed.seedHex,
    seed.isDeriving,
    seed.error,
    config,
    storageRevision,
    latestBurnSummary?.invoiceId,
  ]);

  useEffect(() => {
    let cancelled = false;

    const loadDecimals = async () => {
      if (!qrSelectedToken) {
        if (!cancelled) {
          setQrTokenDecimals(18);
        }
        return;
      }

      try {
        const shouldUseWalletProvider =
          Boolean(wallet.provider) &&
          Boolean(connectedToken) &&
          connectedToken?.chainId === qrSelectedToken.chainId;
        let runner: ContractRunner;
        if (shouldUseWalletProvider) {
          if (!wallet.provider) {
            throw new Error('Wallet provider unavailable');
          }
          runner = wallet.provider;
        } else {
          runner = createProviderForToken(qrSelectedToken);
        }
        const contract = getZerc20Contract(qrSelectedToken.tokenAddress, runner);
        const value = await contract.decimals();
        if (!cancelled) {
          const numeric = Number(value);
          setQrTokenDecimals(Number.isFinite(numeric) && numeric >= 0 ? Math.trunc(numeric) : 18);
        }
      } catch {
        if (!cancelled) {
          setQrTokenDecimals(18);
        }
      }
    };

    void loadDecimals();

    return () => {
      cancelled = true;
    };
  }, [connectedToken, qrSelectedToken, wallet.provider]);

  const handleSubmit = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setStatus(undefined);
      setError(undefined);
      setResult(undefined);
      setLatestBurnError(undefined);

      if (!wallet.account) {
        setError('Connect your wallet to issue invoices.');
        return;
      }
      if (!connectedToken) {
        setError('Switch MetaMask to a network listed in tokens.json.');
        return;
      }

      try {
        setIsSubmitting(true);
        const seedHex = seed.seedHex;
        if (!seedHex) {
          setError(seed.error ?? 'Authorize privacy features by reconnecting your wallet.');
          setStatus(undefined);
          return;
        }

        const token = connectedToken;
        const recipientAddress = normalizeHex(wallet.account);

        setStatus('Preparing invoice artifacts…');
        const stealthClient = await getStealthClient(config);
        const artifacts = await prepareInvoiceIssue({
          client: stealthClient,
          seedHex,
          recipientAddress,
          recipientChainId: token.chainId,
          isBatch,
        });

        setStatus('Requesting signature from MetaMask…');
        const signer = await wallet.ensureSigner();
        const signatureHex = await signer.signMessage(artifacts.signatureMessage);
        const signatureBytes = getBytes(signatureHex);

        setStatus('Submitting invoice to storage…');
        await submitInvoice(stealthClient, artifacts.invoiceId, signatureBytes);
        setStatus(undefined);

        const burnSummary = artifacts.burnAddresses.map((entry) => ({
          subId: entry.subId,
          burnAddress: entry.burnAddress,
        }));

        setResult({
          invoiceId: artifacts.invoiceId,
          signatureHex,
          signatureMessage: artifacts.signatureMessage,
          burnAddresses: artifacts.burnAddresses,
          isBatch,
        });
        setLatestBurnSummary({
          invoiceId: artifacts.invoiceId,
          isBatch,
          burnAddresses: burnSummary,
        });
        setInvoiceReloadRevision((prev) => prev + 1);
        setIsLoadingLatestBurns(false);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setLatestBurnError(message);
        setStatus(undefined);
      } finally {
        setIsSubmitting(false);
      }
    },
    [wallet, connectedToken, isBatch, seed, config],
  );

  useEffect(() => {
    return () => {
      if (copyNoticeTimeoutRef.current !== undefined) {
        window.clearTimeout(copyNoticeTimeoutRef.current);
      }
    };
  }, []);

  const handleCopyBurnAddress = useCallback(
    (event: MouseEvent<HTMLButtonElement>, entry: BurnAddressSummary) => {
      setSelectedBurn(entry);
      selectedBurnRef.current = entry.burnAddress;
      const { clientX, clientY } = event;

      const showCopyNotice = () => {
        if (typeof window === 'undefined') {
          return;
        }
        if (copyNoticeTimeoutRef.current !== undefined) {
          window.clearTimeout(copyNoticeTimeoutRef.current);
        }
        const offsetX = 12;
        const offsetY = 28;
        setCopyNotice({
          left: clientX + offsetX,
          top: clientY - offsetY,
        });
        copyNoticeTimeoutRef.current = window.setTimeout(() => {
          setCopyNotice(null);
          copyNoticeTimeoutRef.current = undefined;
        }, 1200);
      };

      const copyWithClipboardApi = async () => {
        try {
          if (typeof navigator !== 'undefined' && navigator.clipboard) {
            await navigator.clipboard.writeText(entry.burnAddress);
            return true;
          }
        } catch {
          // fall through to fallback copy method
        }
        return false;
      };

      const copyWithFallback = () => {
        if (typeof document === 'undefined') {
          return false;
        }
        const textarea = document.createElement('textarea');
        textarea.value = entry.burnAddress;
        textarea.setAttribute('readonly', '');
        textarea.style.position = 'absolute';
        textarea.style.left = '-9999px';
        document.body.appendChild(textarea);
        textarea.select();
        let success = false;
        try {
          success = document.execCommand('copy');
        } catch {
          success = false;
        }
        document.body.removeChild(textarea);
        return success;
      };

      void copyWithClipboardApi().then((success) => {
        if (success) {
          showCopyNotice();
          return;
        }
        if (copyWithFallback()) {
          showCopyNotice();
          return;
        }
      });
    },
    [],
  );

  const renderBurnAddresses = useCallback(
    (entries: BurnAddressSummary[], isBatchInvoice: boolean) => {
      if (isBatchInvoice) {
        return (
          <ol className="burn-addresses">
            {entries.map((entry) => (
              <li
                key={entry.subId}
                className={selectedBurn?.burnAddress === entry.burnAddress ? 'selected' : undefined}
              >
                <div className="burn-address-row">
                  <strong>#{entry.subId}</strong>
                </div>
                <button
                  type="button"
                  className={`burn-address-copy${
                    selectedBurn?.burnAddress === entry.burnAddress ? ' selected' : ''
                  }`}
                  onClick={(event) => handleCopyBurnAddress(event, entry)}
                  title="Copy burn address"
                >
                  <code className="mono">{entry.burnAddress}</code>
                </button>
              </li>
            ))}
          </ol>
        );
      }

      const [entry] = entries;
      if (!entry) {
        return <p className="hint">No burn address available.</p>;
      }

      return (
        <div
          className={`burn-address-single${
            selectedBurn?.burnAddress === entry.burnAddress ? ' selected' : ''
          }`}
        >
          <button
            type="button"
            className={`burn-address-copy${
              selectedBurn?.burnAddress === entry.burnAddress ? ' selected' : ''
            }`}
            onClick={(event) => handleCopyBurnAddress(event, entry)}
            title="Copy burn address"
          >
            <code className="mono">{entry.burnAddress}</code>
          </button>
        </div>
      );
    },
    [handleCopyBurnAddress, selectedBurn],
  );

  useEffect(() => {
    if (!latestBurnSummary || latestBurnSummary.burnAddresses.length === 0) {
      setSelectedBurn(null);
      setQrAmount('1');
      qrAmountRef.current = '1';
      setQrDataUrl(undefined);
      setQrPayload(undefined);
      setQrError(undefined);
      return;
    }

    setSelectedBurn((current) => {
      const match = current
        ? latestBurnSummary.burnAddresses.find((entry) => entry.burnAddress === current.burnAddress)
        : undefined;
      if (match) {
        if (current && current.burnAddress === match.burnAddress) {
          return current;
        }
        return match;
      }
      return latestBurnSummary.burnAddresses[0] ?? null;
    });
  }, [latestBurnSummary]);

  const generateQrData = useCallback(
    async (value: string, burnAddress: string) => {
      if (value.trim() === '') {
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        setQrError(undefined);
        return;
      }

      if (!qrSelectedToken) {
        setQrError('Select a chain to generate the QR code.');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      let parsedAmount: bigint;
      try {
        parsedAmount = parseUnits(value, qrTokenDecimals);
      } catch {
        setQrError('Unable to encode amount. Use a numeric value like 0.5');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      if (parsedAmount <= 0n) {
        setQrError('Enter a positive amount.');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      const chainSuffix = `@${qrSelectedToken.chainId.toString()}`;
      const payload = `ethereum:${qrSelectedToken.tokenAddress}${chainSuffix}/transfer?address=${burnAddress}&uint256=${parsedAmount.toString()}`;
      try {
        const dataUrl = await toDataURL(payload, { errorCorrectionLevel: 'M', margin: 2, scale: 6 });
        if (qrAmountRef.current !== value) {
          return;
        }
        if (selectedBurnRef.current !== burnAddress) {
          return;
        }
        setQrPayload(payload);
        setQrDataUrl(dataUrl);
        setQrError(undefined);
      } catch {
        if (qrAmountRef.current !== value) {
          return;
        }
        if (selectedBurnRef.current !== burnAddress) {
          return;
        }
        setQrError('Unable to encode amount. Use a numeric value like 0.5');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
      }
    },
    [qrSelectedToken, qrTokenDecimals],
  );

  const handleQrAmountChange = useCallback(
    (value: string) => {
      setQrAmount(value);
      qrAmountRef.current = value;
      setQrError(undefined);

      const burnAddress = selectedBurn?.burnAddress;
      if (!burnAddress) {
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      void generateQrData(value, burnAddress);
    },
    [selectedBurn, generateQrData],
  );

  const handleQrChainChange = useCallback(
    (value: string) => {
      if (value === qrChainId) {
        return;
      }
      setQrChainId(value);
      setQrError(undefined);
      setQrDataUrl(undefined);
      setQrPayload(undefined);
    },
    [qrChainId],
  );

  useEffect(() => {
    selectedBurnRef.current = selectedBurn?.burnAddress;
    if (!selectedBurn) {
      setQrDataUrl(undefined);
      setQrPayload(undefined);
      setQrError(undefined);
      return;
    }
    if (qrAmountRef.current.trim() === '') {
      setQrDataUrl(undefined);
      setQrPayload(undefined);
      setQrError(undefined);
      return;
    }
    void generateQrData(qrAmountRef.current, selectedBurn.burnAddress);
  }, [selectedBurn, generateQrData]);

  const renderLatestBurnSummary = useCallback(() => {
    if (!isWalletConnected) {
      return <p className="hint">Connect your wallet to view burn addresses.</p>;
    }
    if (!isSupportedNetwork) {
      return <p className="error">Unsupported network. Switch MetaMask to a chain defined in tokens.json.</p>;
    }
    if (seed.isDeriving) {
      return <p className="hint">Authorize the seed request in your wallet before loading burn addresses.</p>;
    }
    if (seed.error) {
      return <p className="error">Seed authorization failed: {seed.error}</p>;
    }
    if (!seed.seedHex) {
      return <p className="hint">Authorize the seed request in your wallet to load burn addresses.</p>;
    }
    if (isLoadingLatestBurns) {
      return <p className="hint">Loading latest burn addresses…</p>;
    }
    if (latestBurnError) {
      return <p className="error">{latestBurnError}</p>;
    }
    if (!latestBurnSummary) {
      return <p className="hint">No invoices found for this chain yet.</p>;
    }
    return (
      <>
        {latestBurnSummary.isBatch && (
          <ul className="summary">
            <li>Type: Batch</li>
          </ul>
        )}
        {renderBurnAddresses(latestBurnSummary.burnAddresses, latestBurnSummary.isBatch)}
        <div className="wallet-qr-inline">
          {selectedBurn ? (
            <>
              <div className="qr-preview persistent">
                {qrDataUrl ? (
                  <img src={qrDataUrl} alt="Wallet QR code" />
                ) : (
                  <div className="qr-placeholder">
                    <span className="hint">QR code will appear here.</span>
                  </div>
                )}
              </div>
              <div className="wallet-qr-amount">
                <div className="wallet-qr-inputs">
                  <div className="wallet-qr-field">
                    <label htmlFor="wallet-qr-amount">Amount</label>
                    <input
                      id="wallet-qr-amount"
                      type="number"
                      min="0"
                      step="any"
                      value={qrAmount}
                      onChange={(event) => handleQrAmountChange(event.target.value)}
                      placeholder="Example: 0.25"
                    />
                  </div>
                  <div className="wallet-qr-field">
                    <label htmlFor="wallet-qr-chain">Chain</label>
                    <select
                      id="wallet-qr-chain"
                      value={qrChainId}
                      onChange={(event) => handleQrChainChange(event.target.value)}
                      disabled={availableTokens.length === 0}
                    >
                      {availableTokens.map((token) => {
                        const chainValue = token.chainId.toString();
                        return (
                          <option key={chainValue} value={chainValue}>
                            {token.label} (#{chainValue})
                          </option>
                        );
                      })}
                    </select>
                  </div>
                </div>
              </div>
              {qrError && <p className="error">{qrError}</p>}
            </>
          ) : (
            <p className="hint">Select a burn address to view the QR code.</p>
          )}
        </div>
      </>
    );
  }, [
    isWalletConnected,
    isSupportedNetwork,
    seed.isDeriving,
    seed.error,
    seed.seedHex,
    isLoadingLatestBurns,
    latestBurnError,
    latestBurnSummary,
    renderBurnAddresses,
    selectedBurn,
    qrDataUrl,
    qrAmount,
    qrError,
    handleQrAmountChange,
    handleQrChainChange,
    qrChainId,
    availableTokens,
  ]);

  return (
    <>
      {copyNotice && (
        <div className="copy-toast" style={{ top: copyNotice.top, left: copyNotice.left }}>
          Copied
        </div>
      )}
      <section className="card">
        <header className="card-header">
          <div>
            <h2>Receive</h2>
          </div>
        </header>
        <div className="card-section">
          <h3>Burn Address</h3>
          {renderLatestBurnSummary()}
        </div>
        {availableTokens.length === 0 ? (
          <div className="card-body">
            <p className="error">No token entries were loaded from tokens.json.</p>
          </div>
        ) : (
          <form className="card-body grid" onSubmit={handleSubmit}>
            <footer className="card-footer full">
              <button
                type="submit"
                className="primary"
                disabled={
                  isSubmitting || !isWalletConnected || !isSupportedNetwork || seed.isDeriving || !seed.seedHex
                }
              >
                {isSubmitting ? 'Regenerating…' : 'Regenerate'}
              </button>
              {status && <span>{status}</span>}
              {error && <span className="error">{error}</span>}
            </footer>
            <div className="full">
              <button
                type="button"
                className="ghost"
                aria-expanded={showOptions}
                aria-controls="invoice-mode-options"
                onClick={() => setShowOptions((prev) => !prev)}
              >
                {showOptions ? 'Hide options' : 'More options…'}
              </button>
            </div>

            {showOptions && (
              <div>
                <label htmlFor="invoice-mode-options">Invoice Mode</label>
                <select
                  id="invoice-mode-options"
                  value={isBatch ? 'batch' : 'single'}
                  onChange={(event) => setIsBatch(event.target.value === 'batch')}
                >
                  <option value="single">Single</option>
                  <option value="batch">Batch (10 burn addresses)</option>
                </select>
              </div>
            )}

            {seed.isDeriving && (
              <div className="full">
                <p className="hint">Authorize the seed request in your wallet before issuing invoices.</p>
              </div>
            )}
            {seed.error && (
              <div className="full">
                <p className="error">Seed authorization failed: {seed.error}</p>
              </div>
            )}
          </form>
        )}
      </section>
      <div id="invoice-manager-panel">
        <ScanInvoicesPanel
          config={config}
          tokens={tokens}
          artifacts={artifacts}
          storageRevision={storageRevision}
          reloadRevision={invoiceReloadRevision}
        />
      </div>
    </>
  );
}
