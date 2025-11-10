import { FormEvent, useCallback, useMemo, useRef, useState } from 'react';
import { normalizeHex, prepareInvoiceIssue, submitInvoice, getStealthClientFromConfig } from '@zerc20/sdk';
import { getBytes, parseEther } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';
import { useSeed } from '@/hooks/useSeed';
import { toDataURL } from 'qrcode';
import { ScanInvoicesPanel } from '@features/scanInvoices/ScanInvoicesPanel';

interface PrivateReceivePanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
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
}

export function PrivateReceivePanel({ config, tokens, storageRevision }: PrivateReceivePanelProps): JSX.Element {
  const wallet = useWallet();
  const seed = useSeed();
  const [isBatch, setIsBatch] = useState(false);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<InvoiceResult>();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isQrOpen, setIsQrOpen] = useState(false);
  const [qrBurnAddress, setQrBurnAddress] = useState<string>();
  const [qrAmount, setQrAmount] = useState('');
  const [qrDataUrl, setQrDataUrl] = useState<string>();
  const [qrPayload, setQrPayload] = useState<string>();
  const [qrError, setQrError] = useState<string>();
  const qrAmountRef = useRef('');
  const [isInvoiceManagerOpen, setInvoiceManagerOpen] = useState(false);

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

  const isWalletConnected = Boolean(wallet.account && wallet.chainId);
  const isSupportedNetwork = Boolean(connectedToken);

  const handleSubmit = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setStatus(undefined);
      setError(undefined);
      setResult(undefined);

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
        const stealthClient = await getStealthClientFromConfig({
          icReplicaUrl: config.icReplicaUrl,
          storageCanisterId: config.storageCanisterId,
          keyManagerCanisterId: config.keyManagerCanisterId,
        });
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

        setResult({
          invoiceId: artifacts.invoiceId,
          signatureHex,
          signatureMessage: artifacts.signatureMessage,
          burnAddresses: artifacts.burnAddresses,
        });
        setStatus('Invoice submitted.');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsSubmitting(false);
      }
    },
    [wallet, connectedToken, isBatch, seed, config],
  );

  const openQrModal = useCallback((burnAddress: string) => {
    setQrBurnAddress(burnAddress);
    setQrAmount('');
    qrAmountRef.current = '';
    setQrDataUrl(undefined);
    setQrPayload(undefined);
    setQrError(undefined);
    setIsQrOpen(true);
  }, []);

  const closeQrModal = useCallback(() => {
    setIsQrOpen(false);
    setQrBurnAddress(undefined);
    setQrAmount('');
    qrAmountRef.current = '';
    setQrDataUrl(undefined);
    setQrPayload(undefined);
    setQrError(undefined);
  }, []);

  const handleQrAmountChange = useCallback(
    (value: string) => {
      setQrAmount(value);
      qrAmountRef.current = value;
      setQrError(undefined);

      if (!qrBurnAddress || value.trim() === '') {
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      let weiValue: bigint;
      try {
        weiValue = parseEther(value);
      } catch {
        setQrError('Unable to encode amount. Use a numeric value like 0.5');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      if (weiValue <= 0n) {
        setQrError('Enter a positive amount.');
        setQrDataUrl(undefined);
        setQrPayload(undefined);
        return;
      }

      const payload = `ethereum:${qrBurnAddress}?value=${weiValue.toString()}`;
      toDataURL(payload, { errorCorrectionLevel: 'M', margin: 2, scale: 6 })
        .then((dataUrl: string) => {
          if (qrAmountRef.current !== value) {
            return;
          }
          setQrPayload(payload);
          setQrDataUrl(dataUrl);
          setQrError(undefined);
        })
        .catch(() => {
          if (qrAmountRef.current !== value) {
            return;
          }
          setQrError('Unable to encode amount. Use a numeric value like 0.5');
          setQrDataUrl(undefined);
          setQrPayload(undefined);
        });
    },
    [qrBurnAddress],
  );

  return (
    <>
      <section className="card">
        <header className="card-header">
          <div>
            <h2>Generate Burn Address</h2>
          </div>
        </header>
        {availableTokens.length === 0 ? (
          <div className="card-body">
            <p className="error">No token entries were loaded from tokens.json.</p>
          </div>
        ) : (
          <form className="card-body grid" onSubmit={handleSubmit}>
            <div>
              <label htmlFor="receive-chain">Recipient Chain</label>
              <input
                id="receive-chain"
                type="text"
                value={
                  connectedToken
                    ? `${connectedToken.label} (chain ${connectedToken.chainId.toString()})`
                    : isWalletConnected && wallet.chainId
                    ? `Unsupported (chain ${wallet.chainId})`
                    : ''
                }
                readOnly
                placeholder="Connect MetaMask"
              />
              {!isWalletConnected && <p className="hint">Connect your wallet in MetaMask.</p>}
              {isWalletConnected && !isSupportedNetwork && (
                <p className="error">Unsupported network. Switch MetaMask to a chain defined in tokens.json.</p>
              )}
            </div>
            <div>
              <label htmlFor="receive-mode">Invoice Mode</label>
              <select
                id="receive-mode"
                value={isBatch ? 'batch' : 'single'}
                onChange={(event) => setIsBatch(event.target.value === 'batch')}
              >
                <option value="single">Single</option>
                <option value="batch">Batch (10 burn addresses)</option>
              </select>
            </div>
            <div className="full">
              <label htmlFor="receive-recipient">Recipient Address</label>
              <input
                id="receive-recipient"
                type="text"
                value={wallet.account ?? ''}
                readOnly
                disabled
                placeholder="Connect MetaMask"
              />
              {!wallet.account && <p className="hint">Connect your wallet to populate the recipient address.</p>}
            </div>

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

            <footer className="card-footer full">
              <button
                type="submit"
                className="primary"
                disabled={
                  isSubmitting || !isWalletConnected || !isSupportedNetwork || seed.isDeriving || !seed.seedHex
                }
              >
                {isSubmitting ? 'Submitting…' : 'Issue Invoice'}
              </button>
              {status && <span>{status}</span>}
              {error && <span className="error">{error}</span>}
            </footer>
          </form>
        )}

        {result && (
          <div className="card-section">
            <h3>Invoice Summary</h3>
            <ul className="summary">
              <li>
                Invoice ID: <code className="mono">{result.invoiceId}</code>
              </li>
            </ul>
            <h4>Burn Addresses</h4>
            <ol className="burn-addresses">
              {result.burnAddresses.map((entry) => (
                <li key={entry.subId}>
                  <div className="burn-address-row">
                    <strong>#{entry.subId}</strong>
                    <button type="button" onClick={() => openQrModal(entry.burnAddress)}>
                      QR
                    </button>
                  </div>
                  <code className="mono">{entry.burnAddress}</code>
                </li>
              ))}
            </ol>
          </div>
        )}
        {isQrOpen && qrBurnAddress && (
          <div className="modal-overlay" role="dialog" aria-modal="true">
            <div className="modal">
              <div className="modal-header">
                <h3>Wallet QR</h3>
                <button type="button" className="ghost" onClick={closeQrModal}>
                  Close
                </button>
              </div>
              <div className="modal-body">
                <div>
                  <span className="detail-label">Burn address</span>
                  <span className="detail-value">
                    <code className="mono">{qrBurnAddress}</code>
                  </span>
                </div>
                <div>
                  <label htmlFor="qr-amount">Amount</label>
                  <input
                    id="qr-amount"
                    type="number"
                    min="0"
                    step="any"
                    value={qrAmount}
                    onChange={(event) => handleQrAmountChange(event.target.value)}
                    placeholder="Example: 0.25"
                  />
                </div>
                {qrError && <p className="error">{qrError}</p>}
                {qrDataUrl && qrPayload && (
                  <div className="qr-preview">
                    <img src={qrDataUrl} alt="Wallet QR code" />
                    <code className="mono">{qrPayload}</code>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
        <div className="card-toggle">
          <button
            type="button"
            className={isInvoiceManagerOpen ? 'card-toggle-button open' : 'card-toggle-button'}
            aria-expanded={isInvoiceManagerOpen}
            aria-controls="invoice-manager-panel"
            onClick={() => setInvoiceManagerOpen((prev) => !prev)}
          >
            <span className="card-toggle-label">Manage Issued Invoices</span>
            <span className="card-toggle-icon" aria-hidden="true">
              {isInvoiceManagerOpen ? '−' : '+'}
            </span>
          </button>
        </div>
      </section>
      {isInvoiceManagerOpen && (
        <div id="invoice-manager-panel">
          <ScanInvoicesPanel config={config} tokens={tokens} storageRevision={storageRevision} />
        </div>
      )}
    </>
  );
}
