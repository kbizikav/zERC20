import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import type { FocusEvent, MouseEvent } from 'react';
import { formatUnits, parseUnits } from 'ethers';
import {
  preparePrivateSend,
  submitPrivateSendAnnouncement,
  getZerc20Contract,
  createProviderForToken,
  normalizeHex,
  TokenEntry,
} from '@services/sdk';
import { useWallet } from '@app/providers/WalletProvider';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens } from '@/types/app';
import { getStealthClient } from '@services/clients';
import { useSeed } from '@/hooks/useSeed';
import { getExplorerTxUrl } from '@utils/explorer';

interface PrivateSendPanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
}

interface PrivateSendResultSummary {
  burnAddress: string;
  burnPayload: string;
  announcementId: bigint;
  transactionHash: string;
  chainId: bigint;
}

interface SummaryCopyFieldProps {
  label: string;
  value: string;
  linkHref?: string;
}

function SummaryCopyField({ label, value, linkHref }: SummaryCopyFieldProps): JSX.Element {
  const handleSelect = (event: FocusEvent<HTMLInputElement> | MouseEvent<HTMLInputElement>) => {
    event.currentTarget.select();
  };

  return (
    <li className="summary-item">
      <span className="summary-label">{label}</span>
      {linkHref ? (
        <a className="summary-link mono" href={linkHref} target="_blank" rel="noopener noreferrer">
          {value}
        </a>
      ) : (
        <input
          className="summary-input mono"
          type="text"
          value={value}
          readOnly
          spellCheck={false}
          onFocus={handleSelect}
          onClick={handleSelect}
        />
      )}
    </li>
  );
}

function parseBigNumberish(value: string, label: string): bigint {
  const trimmed = value.trim();
  if (!trimmed) {
    throw new Error(`${label} is required`);
  }
  if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
    return BigInt(trimmed);
  }
  if (!/^\d+$/.test(trimmed)) {
    throw new Error(`${label} must be decimal or 0x-prefixed hex`);
  }
  return BigInt(trimmed);
}

function parseDecimalAmount(value: string, decimals: number, label: string): bigint {
  const trimmed = value.trim();
  if (!trimmed) {
    throw new Error(`${label} is required`);
  }
  if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
    return BigInt(trimmed);
  }
  if (!/^\d+(\.\d+)?$/.test(trimmed)) {
    throw new Error(`${label} must be a decimal number`);
  }
  try {
    return parseUnits(trimmed, decimals);
  } catch {
    const placesLabel = decimals === 1 ? 'decimal place' : 'decimal places';
    throw new Error(`${label} must not exceed ${decimals} ${placesLabel}`);
  }
}

export function PrivateSendPanel({ config, tokens }: PrivateSendPanelProps): JSX.Element {
  const wallet = useWallet();
  const seed = useSeed();
  const [recipientChainId, setRecipientChainId] = useState('');
  const [recipient, setRecipient] = useState('');
  const [amount, setAmount] = useState('');
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<PrivateSendResultSummary>();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [tokenDecimals, setTokenDecimals] = useState<number>(18);
  const [tokenBalance, setTokenBalance] = useState<bigint>();
  const [isBalanceLoading, setIsBalanceLoading] = useState(false);
  const [balanceError, setBalanceError] = useState<string>();

  const availableTokens = useMemo(() => tokens.tokens ?? [], [tokens.tokens]);
  const transactionExplorerUrl = useMemo(
    () => (result ? getExplorerTxUrl(result.chainId, result.transactionHash) : undefined),
    [result],
  );

  const connectedToken = useMemo<TokenEntry | undefined>(() => {
    const chain = wallet.chainId;
    if (chain === undefined || chain === null) {
      return undefined;
    }
    try {
      return availableTokens.find((entry) => entry.chainId === BigInt(chain));
    } catch {
      return undefined;
    }
  }, [availableTokens, wallet.chainId]);

  useEffect(() => {
    if (!connectedToken) {
      setRecipientChainId('');
      return;
    }
    setRecipientChainId((current) => {
      if (!current) {
        return connectedToken.chainId.toString();
      }
      const exists = availableTokens.some((entry) => entry.chainId.toString() === current);
      return exists ? current : connectedToken.chainId.toString();
    });
  }, [availableTokens, connectedToken]);

  useEffect(() => {
    let cancelled = false;

    const loadTokenDecimals = async () => {
      if (!connectedToken) {
        if (!cancelled) {
          setTokenDecimals(18);
        }
        return;
      }

      try {
        const runner = wallet.provider ?? createProviderForToken(connectedToken);
        const contract = getZerc20Contract(connectedToken.tokenAddress, runner);
        const value = await contract.decimals();
        if (!cancelled) {
          const numeric = Number(value);
          setTokenDecimals(Number.isFinite(numeric) && numeric >= 0 ? Math.trunc(numeric) : 18);
        }
      } catch {
        if (!cancelled) {
          setTokenDecimals(18);
        }
      }
    };

    loadTokenDecimals().catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [connectedToken, wallet.provider]);

  useEffect(() => {
    let cancelled = false;
    const account = wallet.account;
    const token = connectedToken;

    if (!account || !token) {
      if (!cancelled) {
        setTokenBalance(undefined);
        setBalanceError(undefined);
        setIsBalanceLoading(false);
      }
      return;
    }

    const loadBalance = async () => {
      setIsBalanceLoading(true);
      setBalanceError(undefined);
      try {
        const runner = wallet.provider ?? createProviderForToken(token);
        const contract = getZerc20Contract(token.tokenAddress, runner);
        const balanceValue = await contract.balanceOf(account);
        if (!cancelled) {
          setTokenBalance(balanceValue);
        }
      } catch (err) {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : String(err);
          setTokenBalance(undefined);
          setBalanceError(`Failed to load balance: ${message}`);
        }
      } finally {
        if (!cancelled) {
          setIsBalanceLoading(false);
        }
      }
    };

    loadBalance().catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [wallet.account, wallet.provider, connectedToken]);

  const isWalletConnected = Boolean(wallet.account && wallet.chainId);
  const isSupportedNetwork = Boolean(connectedToken);
  const decimalsToUse = useMemo(
    () => (Number.isFinite(tokenDecimals) && tokenDecimals >= 0 ? Math.trunc(tokenDecimals) : 18),
    [tokenDecimals],
  );

  const handleSubmit = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setError(undefined);
      setStatus(undefined);
      setResult(undefined);

      if (!wallet.account) {
        setError('Connect your wallet to submit a private transfer.');
        return;
      }
      if (!connectedToken) {
        setError('Switch MetaMask to a network listed in tokens.json.');
        return;
      }
      if (!recipientChainId) {
        setError('Select a recipient chain.');
        return;
      }

      try {
        const parsedAmount = parseDecimalAmount(amount, decimalsToUse, 'Amount');
        if (tokenBalance !== undefined && parsedAmount >= tokenBalance) {
          setError('Amount exceeds available balance.');
          return;
        }

        setIsSubmitting(true);
        const seedHex = seed.seedHex;
        if (!seedHex) {
          setError(seed.error ?? 'Authorize privacy features by reconnecting your wallet.');
          return;
        }

        const token = connectedToken;
        const destinationChain = parseBigNumberish(recipientChainId, 'Recipient chain ID');
        const normalizedRecipient = normalizeHex(recipient);

        setStatus('Preparing encrypted announcement…');
        const stealthClient = await getStealthClient(config);
        const preparation = await preparePrivateSend({
          client: stealthClient,
          recipientAddress: normalizedRecipient,
          recipientChainId: destinationChain,
          seedHex,
        });

        setStatus('Submitting announcement to storage…');
        const announcement = await submitPrivateSendAnnouncement({
          client: stealthClient,
          preparation,
        });

        setStatus('Sending ERC-20 transfer…');
        const signer = await wallet.ensureSigner();
        const contract = getZerc20Contract(token.tokenAddress, signer);
        const tx = await contract.transfer(preparation.burnAddress, parsedAmount);
        const receipt = await tx.wait();

        setResult({
          burnAddress: preparation.burnAddress,
          burnPayload: preparation.burnPayload,
          announcementId: announcement.announcement.id,
          transactionHash: receipt?.hash ?? tx.hash,
          chainId: token.chainId,
        });
        setStatus('Private transfer submitted.');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsSubmitting(false);
      }
    },
    [wallet, connectedToken, recipientChainId, recipient, amount, seed, config, decimalsToUse, tokenBalance],
  );

  return (
    <section className="card">
      <header className="card-header">
        <h2>Private Sending</h2>
      </header>
      {availableTokens.length === 0 ? (
        <div className="card-body">
          <p className="error">No token entries were loaded from tokens.json.</p>
        </div>
      ) : (
        <form className="card-body grid" onSubmit={handleSubmit}>
          <div className="chain-selection">
            <div className="chain-field">
              <label htmlFor="send-recipient-chain">Recipient Chain</label>
              <select
                id="send-recipient-chain"
                required
                value={recipientChainId}
                onChange={(event) => setRecipientChainId(event.target.value)}
              >
                <option value="">Select chain</option>
                {availableTokens.map((token) => (
                  <option key={token.chainId.toString()} value={token.chainId.toString()}>
                    {token.label} (chain {token.chainId.toString()})
                  </option>
                ))}
              </select>
              {!isWalletConnected && <p className="hint">Connect your wallet in MetaMask.</p>}
              {isWalletConnected && !isSupportedNetwork && (
                <p className="error">Unsupported network. Switch MetaMask to a chain defined in tokens.json.</p>
              )}
            </div>
          </div>
          <div className="full">
            <label htmlFor="send-recipient">Recipient Address</label>
            <input
              id="send-recipient"
              type="text"
              placeholder="0x…"
              required
              value={recipient}
              onChange={(event) => setRecipient(event.target.value)}
            />
          </div>
          <div>
            <label htmlFor="send-amount">Amount</label>
            <input
              id="send-amount"
              type="text"
              inputMode="decimal"
              pattern="^[0-9]*([.][0-9]*)?$"
              autoComplete="off"
              spellCheck={false}
              required
              placeholder="e.g. 1.5"
              value={amount}
              onChange={(event) => setAmount(event.target.value)}
            />
            <p className="hint">
              Available:{' '}
              {isBalanceLoading
                ? 'Loading…'
                : tokenBalance !== undefined
                ? formatUnits(tokenBalance, decimalsToUse)
                : '----'}
            </p>
            {balanceError && <p className="error">{balanceError}</p>}
          </div>

          {seed.isDeriving && (
            <div className="full">
              <p className="hint">Authorize the seed request in your wallet to enable private transfers.</p>
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
                isSubmitting || !isSupportedNetwork || !isWalletConnected || seed.isDeriving || !seed.seedHex
              }
            >
              {isSubmitting ? 'Submitting…' : 'Send Private Transfer'}
            </button>
            {status && <span>{status}</span>}
            {error && <span className="error">{error}</span>}
          </footer>
        </form>
      )}

      {result && (
        <div className="card-section">
          <h3>Transfer Summary</h3>
          <ul className="summary summary-copyable">
            <SummaryCopyField label="Burn Address" value={result.burnAddress} />
            <SummaryCopyField label="Burn Payload" value={result.burnPayload} />
            <SummaryCopyField label="Announcement ID" value={result.announcementId.toString()} />
            <SummaryCopyField
              label="Transaction Hash"
              value={result.transactionHash}
              linkHref={transactionExplorerUrl}
            />
          </ul>
        </div>
      )}
    </section>
  );
}
