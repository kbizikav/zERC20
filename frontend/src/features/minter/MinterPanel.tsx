import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { formatUnits, parseUnits } from 'ethers';

import { useWallet } from '@app/providers/WalletProvider';
import type { NormalizedTokens } from '@zerc20/sdk';
import {
  TokenEntry,
  createProviderForToken,
  getErc20Contract,
  getLiquidityManagerContract,
  getZerc20Contract,
  normalizeHex,
  unwrapWithLiquidityManager,
  wrapWithLiquidityManager,
} from '@zerc20/sdk';
import { getExplorerTxUrl } from '@utils/explorer';
import { buildSwitchChainOptions } from '@/utils/wallet';

interface ConvertPanelProps {
  tokens: NormalizedTokens;
  showHeader?: boolean;
}

type ConvertAction = 'deposit' | 'withdraw';

interface ActionResult {
  action: ConvertAction;
  transactionHash: string;
  approvalTransactionHash?: string;
  chainId: bigint;
}

function parseDecimalAmount(value: string, decimals: number, label: string): bigint {
  const trimmed = value.trim();
  if (!trimmed) {
    throw new Error(`${label} is required`);
  }
  if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
    const parsed = BigInt(trimmed);
    if (parsed <= 0n) {
      throw new Error(`${label} must be greater than zero`);
    }
    return parsed;
  }
  if (!/^\d+(\.\d+)?$/.test(trimmed)) {
    throw new Error(`${label} must be a decimal number`);
  }
  try {
    const parsed = parseUnits(trimmed, decimals);
    if (parsed <= 0n) {
      throw new Error(`${label} must be greater than zero`);
    }
    return parsed;
  } catch {
    const placesLabel = decimals === 1 ? 'decimal place' : 'decimal places';
    throw new Error(`${label} must not exceed ${decimals} ${placesLabel}`);
  }
}

export function ConvertPanel({ tokens, showHeader = true }: ConvertPanelProps): JSX.Element {
  const wallet = useWallet();
  const liquidityEnabledTokens = useMemo(
    () => (tokens.tokens ?? []).filter((entry) => Boolean(entry.liquidityManagerAddress)),
    [tokens.tokens],
  );
  const [selectedLabel, setSelectedLabel] = useState<string>(() => liquidityEnabledTokens[0]?.label ?? '');
  const [action, setAction] = useState<ConvertAction>('deposit');
  const [amount, setAmount] = useState('');
  const [underlyingDecimals, setUnderlyingDecimals] = useState<number>(18);
  const [wrappedDecimals, setWrappedDecimals] = useState<number>(18);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<ActionResult>();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [depositBalance, setDepositBalance] = useState<bigint>();
  const [withdrawBalance, setWithdrawBalance] = useState<bigint>();
  const [isDepositBalanceLoading, setIsDepositBalanceLoading] = useState(false);
  const [isWithdrawBalanceLoading, setIsWithdrawBalanceLoading] = useState(false);
  const [depositBalanceError, setDepositBalanceError] = useState<string>();
  const [withdrawBalanceError, setWithdrawBalanceError] = useState<string>();
  const [underlyingTokenAddress, setUnderlyingTokenAddress] = useState<string>();
  const [isUnderlyingTokenLoading, setIsUnderlyingTokenLoading] = useState(false);
  const [underlyingTokenError, setUnderlyingTokenError] = useState<string>();
  const [amountBalanceError, setAmountBalanceError] = useState<string>();
  const [switchingChainId, setSwitchingChainId] = useState<bigint>();
  const [switchError, setSwitchError] = useState<string>();

  const selectedToken = useMemo<TokenEntry | undefined>(
    () => liquidityEnabledTokens.find((entry) => entry.label === selectedLabel),
    [liquidityEnabledTokens, selectedLabel],
  );
  const walletChainId = wallet.chainId !== undefined ? BigInt(wallet.chainId) : undefined;
  const isWalletOnSupportedLiquidityChain = useMemo(() => {
    if (walletChainId === undefined) {
      return false;
    }
    return liquidityEnabledTokens.some((entry) => entry.chainId === walletChainId);
  }, [liquidityEnabledTokens, walletChainId]);
  const shouldSuggestNetworkSwitch =
    walletChainId !== undefined &&
    !isWalletOnSupportedLiquidityChain &&
    liquidityEnabledTokens.length > 0;

  const resolveRunner = useCallback(
    (token: TokenEntry) => {
      if (wallet.chainId !== undefined && BigInt(wallet.chainId) === token.chainId && wallet.publicClient) {
        return wallet.publicClient;
      }
      return createProviderForToken(token);
    },
    [wallet.chainId, wallet.publicClient],
  );

  const underlyingDecimalsToUse = useMemo(
    () =>
      Number.isFinite(underlyingDecimals) && underlyingDecimals >= 0
        ? Math.trunc(underlyingDecimals)
        : 18,
    [underlyingDecimals],
  );

  const wrappedDecimalsToUse = useMemo(
    () =>
      Number.isFinite(wrappedDecimals) && wrappedDecimals >= 0 ? Math.trunc(wrappedDecimals) : 18,
    [wrappedDecimals],
  );

  const activeDecimals = action === 'deposit' ? underlyingDecimalsToUse : wrappedDecimalsToUse;

  const activeBalance = action === 'deposit' ? depositBalance : withdrawBalance;
  const isActiveBalanceLoading =
    action === 'deposit' ? isDepositBalanceLoading : isWithdrawBalanceLoading;
  const activeBalanceError = action === 'deposit' ? depositBalanceError : withdrawBalanceError;
  const transactionExplorerUrl = result ? getExplorerTxUrl(result.chainId, result.transactionHash) : undefined;
  const approvalExplorerUrl =
    result?.approvalTransactionHash
      ? getExplorerTxUrl(result.chainId, result.approvalTransactionHash)
      : undefined;

  useEffect(() => {
    if (!shouldSuggestNetworkSwitch) {
      setSwitchError(undefined);
      setSwitchingChainId(undefined);
    }
  }, [shouldSuggestNetworkSwitch]);

  useEffect(() => {
    if (liquidityEnabledTokens.length === 0) {
      setSelectedLabel('');
      return;
    }
    if (!selectedToken) {
      setSelectedLabel(liquidityEnabledTokens[0]?.label ?? '');
    }
  }, [liquidityEnabledTokens, selectedToken]);

  useEffect(() => {
    let cancelled = false;
    const token = selectedToken;
    if (!token || !token.liquidityManagerAddress) {
      setUnderlyingTokenAddress(undefined);
      setUnderlyingTokenError(undefined);
      setIsUnderlyingTokenLoading(false);
      return;
    }

    const managerAddress = token.liquidityManagerAddress;

    const loadUnderlyingTokenAddress = async () => {
      setIsUnderlyingTokenLoading(true);
      setUnderlyingTokenError(undefined);
      setUnderlyingTokenAddress(undefined);
      try {
        const runner = resolveRunner(token);
        const contract = getLiquidityManagerContract(managerAddress, runner);
        const address = (await contract.read.underlyingToken()) as string;
        if (!cancelled) {
          setUnderlyingTokenAddress(normalizeHex(address));
        }
      } catch (err) {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : String(err);
          setUnderlyingTokenAddress(undefined);
          setUnderlyingTokenError(`Failed to load convert token configuration: ${message}`);
        }
      } finally {
        if (!cancelled) {
          setIsUnderlyingTokenLoading(false);
        }
      }
    };

    loadUnderlyingTokenAddress().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [resolveRunner, selectedToken]);

  useEffect(() => {
    let cancelled = false;
    const token = selectedToken;

    if (!token) {
      setWrappedDecimals(18);
      return;
    }

    const loadWrappedDecimals = async () => {
      try {
        const runner = resolveRunner(token);
        const contract = getZerc20Contract(token.tokenAddress, runner);
        const value = (await contract.read.decimals()) as bigint | number;
        if (!cancelled) {
          const numeric = Number(value);
          setWrappedDecimals(Number.isFinite(numeric) && numeric >= 0 ? Math.trunc(numeric) : 18);
        }
      } catch {
        if (!cancelled) {
          setWrappedDecimals(18);
        }
      }
    };

    loadWrappedDecimals().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [resolveRunner, selectedToken]);

  useEffect(() => {
    let cancelled = false;
    const token = selectedToken;

    if (!token || !underlyingTokenAddress) {
      setUnderlyingDecimals(18);
      return;
    }

    const loadUnderlyingDecimals = async () => {
      try {
        const runner = resolveRunner(token);
        const contract = getErc20Contract(underlyingTokenAddress, runner);
        const value = (await contract.read.decimals()) as bigint | number;
        if (!cancelled) {
          const numeric = Number(value);
          setUnderlyingDecimals(Number.isFinite(numeric) && numeric >= 0 ? Math.trunc(numeric) : 18);
        }
      } catch {
        if (!cancelled) {
          setUnderlyingDecimals(18);
        }
      }
    };

    loadUnderlyingDecimals().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [resolveRunner, selectedToken, underlyingTokenAddress]);

  useEffect(() => {
    let cancelled = false;
    const account = wallet.account;
    const token = selectedToken;

    if (!account || !token || !underlyingTokenAddress) {
      if (!cancelled) {
        setDepositBalance(undefined);
        setDepositBalanceError(undefined);
        setIsDepositBalanceLoading(false);
      }
      return;
    }

    const loadDepositBalance = async () => {
      setIsDepositBalanceLoading(true);
      setDepositBalanceError(undefined);
      try {
        const runner = resolveRunner(token);
        const contract = getErc20Contract(underlyingTokenAddress, runner);
        const rawBalance = (await contract.read.balanceOf([account as `0x${string}`])) as bigint;
        if (!cancelled) {
          setDepositBalance(rawBalance);
        }
      } catch (err) {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : String(err);
          setDepositBalance(undefined);
          setDepositBalanceError(`Failed to load deposit balance: ${message}`);
        }
      } finally {
        if (!cancelled) {
          setIsDepositBalanceLoading(false);
        }
      }
    };

    loadDepositBalance().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [resolveRunner, selectedToken, underlyingTokenAddress, wallet.account]);

  useEffect(() => {
    let cancelled = false;
    const account = wallet.account;
    const token = selectedToken;

    if (!account || !token) {
      if (!cancelled) {
        setWithdrawBalance(undefined);
        setWithdrawBalanceError(undefined);
        setIsWithdrawBalanceLoading(false);
      }
      return;
    }

    const loadWithdrawBalance = async () => {
      setIsWithdrawBalanceLoading(true);
      setWithdrawBalanceError(undefined);
      try {
        const runner = resolveRunner(token);
        const contract = getZerc20Contract(token.tokenAddress, runner);
        const rawBalance = (await contract.read.balanceOf([account as `0x${string}`])) as bigint;
        if (!cancelled) {
          setWithdrawBalance(rawBalance);
        }
      } catch (err) {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : String(err);
          setWithdrawBalance(undefined);
          setWithdrawBalanceError(`Failed to load withdraw balance: ${message}`);
        }
      } finally {
        if (!cancelled) {
          setIsWithdrawBalanceLoading(false);
        }
      }
    };

    loadWithdrawBalance().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [resolveRunner, selectedToken, wallet.account]);

  useEffect(() => {
    setAmountBalanceError(undefined);
    const trimmed = amount.trim();
    if (!trimmed) {
      return;
    }

    const available = action === 'deposit' ? depositBalance : withdrawBalance;
    if (available === undefined) {
      return;
    }

    const decimalsForValidation =
      action === 'deposit' ? underlyingDecimalsToUse : wrappedDecimalsToUse;
    try {
      const parsed = parseDecimalAmount(trimmed, decimalsForValidation, 'Amount');
      if (parsed > available) {
        setAmountBalanceError('Amount exceeds available balance.');
      }
    } catch {
      // Ignore parsing errors here; submission will handle invalid input.
    }
  }, [
    action,
    amount,
    depositBalance,
    underlyingDecimalsToUse,
    withdrawBalance,
    wrappedDecimalsToUse,
  ]);

  const handleSubmit = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setError(undefined);
      setStatus(undefined);
      setResult(undefined);

      const token = selectedToken;
      if (!token || !token.liquidityManagerAddress) {
        setError('Select a token configured with a convert contract.');
        return;
      }

      if (!underlyingTokenAddress) {
        setError(underlyingTokenError ?? 'Unable to determine the convert token configuration.');
        return;
      }

      let parsedAmount: bigint;
      try {
        parsedAmount = parseDecimalAmount(amount, activeDecimals, 'Amount');
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
        return;
      }

      const availableBalance = action === 'deposit' ? depositBalance : withdrawBalance;
      if (availableBalance !== undefined && parsedAmount > availableBalance) {
        const message = 'Amount exceeds available balance.';
        setError(message);
        setAmountBalanceError(message);
        return;
      }

      try {
        setIsSubmitting(true);
        const expectedChainId = token.chainId;

        if (!wallet.chainId || BigInt(wallet.chainId) !== expectedChainId) {
          setStatus('Switching network in wallet…');
          const switchOptions = buildSwitchChainOptions(token);
          await wallet.switchChain(expectedChainId, switchOptions);
        }

        const walletClient = await wallet.ensureWalletClient();
        const params = {
          walletClient,
          publicClient: wallet.publicClient,
          liquidityManagerAddress: token.liquidityManagerAddress,
          amount: parsedAmount,
        };

        if (action === 'deposit') {
          setStatus('Submitting approval (if needed) and wrap transaction…');
          const outcome = await wrapWithLiquidityManager(params);
          setResult({
            action,
            transactionHash: outcome.transactionHash,
            approvalTransactionHash: outcome.approvalTransactionHash,
            chainId: token.chainId,
          });
          setStatus(
            outcome.approvalTransactionHash
              ? 'Wrap submitted after approval.'
              : 'Wrap transaction submitted.',
          );
        } else {
          setStatus('Submitting unwrap transaction…');
          const outcome = await unwrapWithLiquidityManager(params);
          setResult({
            action,
            transactionHash: outcome.transactionHash,
            chainId: token.chainId,
          });
          setStatus('Unwrap transaction submitted.');
        }
        setAmount('');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsSubmitting(false);
      }
    },
    [
      action,
      activeDecimals,
      amount,
      depositBalance,
      underlyingTokenAddress,
      underlyingTokenError,
      selectedToken,
      wallet,
      withdrawBalance,
    ],
  );

  const handleSwitchChainRequest = useCallback(
    async (targetToken: TokenEntry) => {
      try {
        setSwitchError(undefined);
        setSwitchingChainId(targetToken.chainId);
        const switchOptions = buildSwitchChainOptions(targetToken);
        await wallet.switchChain(targetToken.chainId, switchOptions);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setSwitchError(message);
      } finally {
        setSwitchingChainId(undefined);
      }
    },
    [wallet],
  );

  return (
    <section className="card">
      {showHeader && (
        <header className="card-header">
          <h2>Convert</h2>
        </header>
      )}
      {liquidityEnabledTokens.length === 0 ? (
        <div className="card-body">
          <p className="error">No convert-enabled tokens were loaded from tokens.json.</p>
        </div>
      ) : (
        <form className="card-body grid" onSubmit={handleSubmit}>
          {shouldSuggestNetworkSwitch && (
            <div className="full">
              <p className="info">
                Connected network (chain {walletChainId?.toString()}) is not configured with a convert
                contract. Switch to one of the supported networks below.
              </p>
              <ul className="summary">
                {liquidityEnabledTokens.map((token) => (
                  <li key={token.label}>
                    <button
                      type="button"
                      className="outline"
                      disabled={switchingChainId !== undefined}
                      onClick={() => void handleSwitchChainRequest(token)}
                    >
                      {switchingChainId === token.chainId ? 'Switching…' : `Switch to ${token.label} (chain ${token.chainId.toString()})`}
                    </button>
                  </li>
                ))}
              </ul>
              {switchError && <p className="error">{switchError}</p>}
            </div>
          )}
          <div>
            <label htmlFor="convert-token">Token</label>
            <select
              id="convert-token"
              required
              value={selectedToken?.label ?? ''}
              onChange={(event) => setSelectedLabel(event.target.value)}
            >
              {liquidityEnabledTokens.map((token) => (
                <option key={token.label} value={token.label}>
                  {token.label} (chain {token.chainId.toString()})
                </option>
              ))}
            </select>
          </div>
          <div>
            <fieldset>
              <legend>Action</legend>
              <label>
                <input
                  type="radio"
                  name="convert-action"
                  value="deposit"
                  checked={action === 'deposit'}
                  onChange={() => setAction('deposit')}
                />
                Deposit
              </label>
              <label>
                <input
                  type="radio"
                  name="convert-action"
                  value="withdraw"
                  checked={action === 'withdraw'}
                  onChange={() => setAction('withdraw')}
                />
                Withdraw
              </label>
            </fieldset>
          </div>
          <div className="full">
            <label htmlFor="convert-amount">Amount</label>
            <input
              id="convert-amount"
              type="text"
              inputMode="decimal"
              pattern="^[0-9]*([.][0-9]*)?$"
              autoComplete="off"
              spellCheck={false}
              placeholder="e.g. 1.0"
              value={amount}
              onChange={(event) => setAmount(event.target.value)}
            />
            {isUnderlyingTokenLoading && <p className="hint">Loading convert configuration…</p>}
            {underlyingTokenError && <p className="error">{underlyingTokenError}</p>}
            {amountBalanceError && <p className="error">{amountBalanceError}</p>}
            <p className="hint">
              Available:{' '}
              {isActiveBalanceLoading
                ? 'Loading…'
                : activeBalance !== undefined
                ? formatUnits(activeBalance, activeDecimals)
                : '----'}
            </p>
            {activeBalanceError && <p className="error">{activeBalanceError}</p>}
          </div>

          <footer className="card-footer full">
            <button
              type="submit"
              className="primary"
              disabled={
                isSubmitting ||
                !selectedToken?.liquidityManagerAddress ||
                isUnderlyingTokenLoading ||
                !underlyingTokenAddress ||
                Boolean(amountBalanceError)
              }
            >
              {isSubmitting
                ? 'Submitting…'
                : action === 'deposit'
                ? 'Deposit Tokens'
                : 'Withdraw Tokens'}
            </button>
            {status && <span>{status}</span>}
            {error && <span className="error">{error}</span>}
          </footer>
        </form>
      )}

      {result && (
        <div className="card-section">
          <ul className="summary summary-copyable">
            <li>
              <span className="mono">
                Action: {result.action === 'deposit' ? 'Deposit' : 'Withdraw'}
              </span>
            </li>
            {result.approvalTransactionHash && (
              <li>
                <span className="mono">
                  Approval TX HASH:{' '}
                  {approvalExplorerUrl ? (
                    <a
                      className="summary-link mono"
                      href={approvalExplorerUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      {result.approvalTransactionHash}
                    </a>
                  ) : (
                    result.approvalTransactionHash
                  )}
                </span>
              </li>
            )}
            <li>
              <span className="mono">
                TX HASH:{' '}
                {transactionExplorerUrl ? (
                  <a
                    className="summary-link mono"
                    href={transactionExplorerUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    {result.transactionHash}
                  </a>
                ) : (
                  result.transactionHash
                )}
              </span>
            </li>
          </ul>
        </div>
      )}
    </section>
  );
}
