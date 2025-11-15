import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import {
  BurnArtifacts,
  RedeemContext,
  collectRedeemContext,
  deriveInvoiceBatch,
  deriveInvoiceSingle,
  buildFullBurnAddress,
  listInvoices,
  extractChainIdFromInvoiceHex,
  normalizeHex,
  getVerifierContract,
  createProviderForToken,
  getStealthClientFromConfig,
  getDeciderClient,
} from '@zerc20/sdk';
import { formatUnits, getBytes, hexlify, zeroPadValue } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';
import { useSeed } from '@/hooks/useSeed';
import { RedeemDetailSection } from '@features/redeem/RedeemDetailSection';
import { RedeemProgressModal } from '@features/redeem/RedeemProgressModal';
import { createRedeemSteps, setStepStatus, type RedeemStage, type RedeemStep } from '@features/redeem/redeemSteps';
import { yieldToUi } from '@features/redeem/yieldToUi';
import { toAccountKey } from '@utils/accountKey';
import { useStorageStore } from '@/state/storageStore';
import { generateBatchTeleportProof, generateSingleTeleportProof } from '@/utils/teleportProofs';

interface ScanInvoicesPanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
}

interface BurnDetail {
  subId: number;
  burn: BurnArtifacts;
  context: RedeemContext;
}

interface InvoiceDetail {
  invoiceId: string;
  isBatch: boolean;
  burns: BurnDetail[];
  owner: string;
  chainId: bigint;
}

function isSingleInvoice(invoiceId: string): boolean {
  const bytes = getBytes(normalizeHex(invoiceId));
  return (bytes[0] & 0x80) !== 0;
}

function padRecipient(address: string): string {
  return zeroPadValue(normalizeHex(address), 32);
}

function formatEligibleValue(value: bigint): string {
  return formatUnits(value, 18);
}

export function ScanInvoicesPanel({ config, tokens }: ScanInvoicesPanelProps): JSX.Element {
  const wallet = useWallet();
  const seed = useSeed();
  const [selectedInvoice, setSelectedInvoice] = useState<string>();
  const [detail, setDetail] = useState<InvoiceDetail | null>(null);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [isLoading, setIsLoading] = useState(false);
  const [isDetailLoading, setIsDetailLoading] = useState(false);
  const [redeemMessage, setRedeemMessage] = useState<string>();
  const [redeemSteps, setRedeemSteps] = useState<RedeemStep[]>([]);
  const [isRedeemModalOpen, setRedeemModalOpen] = useState(false);

  const availableTokens = useMemo(() => tokens.tokens ?? [], [tokens.tokens]);
  const accountKey = useMemo(() => toAccountKey(wallet.account), [wallet.account]);
  const storedInvoices = useStorageStore((state) => (accountKey ? state.invoices[accountKey] : undefined));
  const setStoredInvoices = useStorageStore((state) => state.setInvoices);
  const connectedToken = useMemo(() => {
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

  const isWalletConnected = Boolean(wallet.account && wallet.chainId);
  const isSupportedNetwork = Boolean(connectedToken);

  const invoices = useMemo(() => {
    if (!accountKey || !connectedToken) {
      return [];
    }
    const invoicesForAccount = storedInvoices ?? [];
    return invoicesForAccount.filter((invoiceId) => {
      try {
        return extractChainIdFromInvoiceHex(invoiceId) === connectedToken.chainId;
      } catch {
        return false;
      }
    });
  }, [accountKey, connectedToken, storedInvoices]);

  useEffect(() => {
    setSelectedInvoice(undefined);
    setDetail(null);
    setIsDetailLoading(false);
  }, [accountKey, connectedToken]);

  useEffect(() => {
    if (!selectedInvoice) {
      return;
    }
    if (!invoices.includes(selectedInvoice)) {
      setSelectedInvoice(undefined);
      setDetail(null);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);
    }
  }, [invoices, selectedInvoice]);

  const mergeInvoices = useCallback(
    (accountAddress: string, invoiceIds: string[], chainId: bigint): { added: number; total: number } => {
      const normalizedAccount = normalizeHex(accountAddress);
      const prev = useStorageStore.getState().invoices[normalizedAccount] ?? [];
      const filteredPrev = prev.filter((invoiceId) => {
        try {
          return extractChainIdFromInvoiceHex(invoiceId) === chainId;
        } catch {
          return false;
        }
      });
      const existing = new Set(filteredPrev);
      const next = [...filteredPrev];
      let added = 0;
      for (const id of invoiceIds) {
        if (!existing.has(id)) {
          existing.add(id);
          next.push(id);
          added += 1;
        }
      }
      setStoredInvoices(normalizedAccount, next);
      return { added, total: next.length };
    },
    [setStoredInvoices],
  );

  const handleLoadInvoices = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setError(undefined);
      setStatus(undefined);
      setDetail(null);
      setIsDetailLoading(false);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);

      if (!wallet.account) {
        setError('Connect your wallet to load invoices.');
        return;
      }
      if (!connectedToken) {
        setError('Switch MetaMask to a network listed in tokens.json.');
        return;
      }

      try {
        setIsLoading(true);
        setStatus('Scanning invoices…');
        const owner = normalizeHex(wallet.account);
        const stealthClient = await getStealthClientFromConfig({
          icReplicaUrl: config.icReplicaUrl,
          storageCanisterId: config.storageCanisterId,
          keyManagerCanisterId: config.keyManagerCanisterId,
        });
        const ids = await listInvoices(stealthClient, owner, connectedToken.chainId);
        const normalizedIds = ids.map((value) => normalizeHex(value));
        const { added, total } = mergeInvoices(owner, normalizedIds, connectedToken.chainId);
        if (total === 0) {
          setStatus(`No invoices were found for ${owner}.`);
        } else if (added === 0) {
          setStatus(`No new invoices. Stored ${total} invoice(s) for ${owner}.`);
        } else {
          setStatus(`Added ${added} new invoice(s). Stored ${total} invoice(s) for ${owner}.`);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsLoading(false);
      }
    },
    [wallet, connectedToken, config, mergeInvoices],
  );

  const handleInvoiceClick = useCallback(
    async (invoiceId: string) => {
      if (selectedInvoice === invoiceId) {
        setSelectedInvoice(undefined);
        setRedeemMessage(undefined);
        setRedeemSteps([]);
        setRedeemModalOpen(false);
        return;
      }

      if (!tokens.hub) {
        setError('Hub configuration is required to inspect invoices.');
        return;
      }
      if (!wallet.account) {
        setError('Connect your wallet to inspect invoices.');
        return;
      }
      if (!connectedToken) {
        setError('Switch MetaMask to a network listed in tokens.json.');
        return;
      }

      setSelectedInvoice(invoiceId);
      setRedeemMessage(undefined);
      setError(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);

      if (detail && detail.invoiceId === invoiceId) {
        return;
      }

      setDetail(null);
      setIsDetailLoading(true);

      try {
        const owner = normalizeHex(wallet.account);
        const seedHex = seed.seedHex;
        if (!seedHex) {
          setError(seed.error ?? 'Authorize privacy features by reconnecting your wallet.');
          setStatus(undefined);
          return;
        }
        setStatus('Fetching invoice status…');

        const burnDetails: BurnDetail[] = [];
        const invoiceIsSingle = isSingleInvoice(invoiceId);
        const subIds = invoiceIsSingle ? [0] : Array.from({ length: 10 }, (_, idx) => idx);

        const tokenProvider = createProviderForToken(connectedToken);
        const verifierContract = getVerifierContract(
          connectedToken.verifierAddress,
          tokenProvider,
        ) as unknown as Parameters<typeof collectRedeemContext>[0]['verifierContract'];

        for (const subId of subIds) {
          const secretAndTweak = invoiceIsSingle
            ? await deriveInvoiceSingle(seedHex, invoiceId, connectedToken.chainId, owner)
            : await deriveInvoiceBatch(seedHex, invoiceId, subId, connectedToken.chainId, owner);
          const burn = await buildFullBurnAddress(
            connectedToken.chainId,
            owner,
            secretAndTweak.secret,
            secretAndTweak.tweak,
          );

          const context = await collectRedeemContext({
            burn,
            tokens: availableTokens,
            hub: tokens.hub,
            verifierContract,
            indexerUrl: config.indexerUrl,
            indexerFetchLimit: config.indexerFetchLimit,
            eventBlockSpan: BigInt(config.eventBlockSpan),
          });

          burnDetails.push({
            subId,
            burn,
            context,
          });
        }

        setDetail({
          invoiceId,
          isBatch: !invoiceIsSingle,
          burns: burnDetails,
          owner,
          chainId: connectedToken.chainId,
        });
        setStatus('Invoice status loaded.');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsDetailLoading(false);
      }
    },
    [tokens, availableTokens, wallet, seed, config, connectedToken, selectedInvoice, detail],
  );

  const redeemBurn = useCallback(
    async (burnDetail: BurnDetail) => {
      if (!detail) {
        return;
      }
      if (!wallet.account) {
        setRedeemMessage('Connect and unlock your wallet before redeeming.');
        return;
      }
      if (!tokens.hub) {
        setRedeemMessage('Hub configuration is required before redeeming.');
        return;
      }

      const initialSteps = createRedeemSteps(burnDetail.context.events.eligible.length > 1);
      setRedeemSteps(initialSteps);
      setRedeemModalOpen(true);
      setRedeemMessage(undefined);
      await yieldToUi();

      let currentStage: RedeemStage | null = null;
      const activateStage = (stage: RedeemStage, message: string) => {
        currentStage = stage;
        setRedeemSteps((prev) => setStepStatus(prev, stage, 'active'));
        setRedeemMessage(message);
      };
      const completeStage = (stage: RedeemStage) => {
        setRedeemSteps((prev) => setStepStatus(prev, stage, 'done'));
        if (currentStage === stage) {
          currentStage = null;
        }
      };
      const failStage = (stage: RedeemStage) => {
        setRedeemSteps((prev) => setStepStatus(prev, stage, 'error'));
        currentStage = stage;
      };

      try {
        setIsLoading(true);
        activateStage('indexer', 'Fetching events from indexer…');
        await yieldToUi();

        const tokenEntry = burnDetail.context.token;
        const tokenProvider = createProviderForToken(tokenEntry);
        const verifierRead = getVerifierContract(
          tokenEntry.verifierAddress,
          tokenProvider,
        ) as unknown as Parameters<typeof collectRedeemContext>[0]['verifierContract'];

        const refreshedContext = await collectRedeemContext({
          burn: burnDetail.burn,
          tokens: availableTokens,
          hub: tokens.hub,
          verifierContract: verifierRead,
          indexerUrl: config.indexerUrl,
          indexerFetchLimit: config.indexerFetchLimit,
          eventBlockSpan: BigInt(config.eventBlockSpan),
        });

        completeStage('indexer');
        await yieldToUi();

        setDetail((current) => {
          if (!current) {
            return current;
          }
          return {
            ...current,
            burns: current.burns.map((item) =>
              item.subId === burnDetail.subId ? { ...item, context: refreshedContext } : item,
            ),
          };
        });
        await yieldToUi();

        if (refreshedContext.totalTeleported >= refreshedContext.totalIndexedValue) {
          setRedeemMessage('This burn has already been redeemed.');
          setRedeemModalOpen(false);
          await yieldToUi();
          return;
        }

        const eligible = refreshedContext.events.eligible;
        if (eligible.length === 0) {
          setRedeemMessage('No eligible transfers to redeem.');
          setRedeemModalOpen(false);
          await yieldToUi();
          return;
        }

        const isBatchRedeem = eligible.length > 1;
        setRedeemSteps((prev) => {
          const hasDecider = prev.some((step) => step.id === 'decider');
          if (hasDecider === isBatchRedeem) {
            return prev;
          }
          const next = createRedeemSteps(isBatchRedeem);
          return setStepStatus(next, 'indexer', 'done');
        });
        await yieldToUi();

        const walletClient = await wallet.ensureWalletClient();
        const verifierWithSigner = getVerifierContract(refreshedContext.token.verifierAddress, walletClient);
        const gr = {
          chainId: burnDetail.burn.generalRecipient.chainId,
          recipient: padRecipient(burnDetail.burn.generalRecipient.address),
          tweak: burnDetail.burn.generalRecipient.tweak,
        };

        activateStage('proof', 'Generating WASM proof…');
        await yieldToUi();

        if (eligible.length === 1) {
          const singleProof = await generateSingleTeleportProof({
            aggregationState: refreshedContext.aggregationState,
            recipientFr: burnDetail.burn.generalRecipient.fr,
            secretHex: burnDetail.burn.secret,
            event: eligible[0],
            proof: refreshedContext.globalProofs[0],
          });

          completeStage('proof');
          await yieldToUi();
          activateStage('wallet', 'Submitting wallet transaction…');
          await yieldToUi();

          const singleTeleport = verifierWithSigner.write.singleTeleport as (
            args: readonly [boolean, bigint, typeof gr, `0x${string}`],
          ) => Promise<`0x${string}`>;
          const proofCalldata = singleProof.proofCalldata as `0x${string}`;
          const txHash = await singleTeleport([
            true,
            refreshedContext.aggregationState.latestAggSeq,
            gr,
            proofCalldata,
          ]);
          const receiptClient = wallet.publicClient ?? createProviderForToken(refreshedContext.token);
          await receiptClient.waitForTransactionReceipt({ hash: txHash });
          completeStage('wallet');
          await yieldToUi();
          setRedeemMessage(`Single teleport submitted: ${txHash}`);
        } else {
          const decider = getDeciderClient({ baseUrl: config.deciderUrl });
          const batchProofInputs = refreshedContext.globalProofs;
          if (batchProofInputs.length !== eligible.length) {
            throw new Error('Indexer returned mismatched event/proof counts for batch teleport.');
          }
          const batchProof = await generateBatchTeleportProof({
            aggregationState: refreshedContext.aggregationState,
            recipientFr: burnDetail.burn.generalRecipient.fr,
            secretHex: burnDetail.burn.secret,
            events: batchProofInputs.map((entry) => entry.event),
            proofs: batchProofInputs,
            decider,
            onDeciderRequestStart: async () => {
              completeStage('proof');
              await yieldToUi();
              activateStage('decider', 'Requesting decider proof…');
              await yieldToUi();
            },
          });

          completeStage('decider');
          await yieldToUi();
          activateStage('wallet', 'Submitting wallet transaction…');
          await yieldToUi();

          const teleport = verifierWithSigner.write.teleport as (
            args: readonly [boolean, bigint, typeof gr, `0x${string}`],
          ) => Promise<`0x${string}`>;
          const deciderProofHex = hexlify(batchProof.deciderProof) as `0x${string}`;
          const txHash = await teleport([
            true,
            refreshedContext.aggregationState.latestAggSeq,
            gr,
            deciderProofHex,
          ]);
          const receiptClient = wallet.publicClient ?? createProviderForToken(refreshedContext.token);
          await receiptClient.waitForTransactionReceipt({ hash: txHash });
          completeStage('wallet');
          await yieldToUi();
          setRedeemMessage(`Batch teleport submitted: ${txHash}`);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        const stageToFail = currentStage ?? 'wallet';
        failStage(stageToFail);
        await yieldToUi();
        setRedeemMessage(`Redeem failed: ${message}`);
      } finally {
        setIsLoading(false);
      }
    },
    [availableTokens, config, detail, tokens, wallet],
  );

  return (
    <section className="card">
      <header className="card-header">
        <h2>Invoices</h2>
        <p className="hint">
          Mirrors <code>cli invoice ls</code> and provides status/redeem flows.
        </p>
      </header>
      {availableTokens.length === 0 ? (
        <div className="card-body">
          <p className="error">No token entries were loaded from tokens.json.</p>
        </div>
      ) : (
        <form className="card-body" onSubmit={handleLoadInvoices}>
          {!isWalletConnected && (
            <p className="hint">Connect your wallet in MetaMask to enable scanning.</p>
          )}
          {isWalletConnected && !isSupportedNetwork && (
            <p className="error">Unsupported network. Switch MetaMask to a chain defined in tokens.json.</p>
          )}
          {seed.isDeriving && (
            <div className="full">
              <p className="hint">Authorize the seed request in your wallet before inspecting invoices.</p>
            </div>
          )}
          {seed.error && (
            <div className="full">
              <p className="error">Seed authorization failed: {seed.error}</p>
            </div>
          )}
          <footer className="card-footer full card-footer-spread">
            <div className="card-footer-status">
              {status && <span>{status}</span>}
              {error && <span className="error">{error}</span>}
            </div>
            <div className="card-footer-actions">
              <button
                type="submit"
                className="primary"
                disabled={isLoading || isDetailLoading || !isWalletConnected || !isSupportedNetwork}
              >
                {isLoading ? 'Scanning…' : 'Scan'}
              </button>
            </div>
          </footer>
        </form>
      )}

      {invoices.length > 0 && (
        <div className="card-section">
          <h3>Invoices</h3>
          <ul className="list accordion">
            {invoices.map((invoiceId) => {
              const isSelected = invoiceId === selectedInvoice;
              const invoiceDetail =
                detail && detail.invoiceId === invoiceId ? detail : null;
              const eligibleTotal = invoiceDetail
                ? formatEligibleValue(
                    invoiceDetail.burns.reduce<bigint>(
                      (acc, burn) => acc + burn.context.totalEligibleValue,
                      0n,
                    ),
                  )
                : '0';
              const pendingTotal = invoiceDetail
                ? formatEligibleValue(
                    invoiceDetail.burns.reduce<bigint>(
                      (acc, burn) => acc + burn.context.totalPendingValue,
                      0n,
                    ),
                  )
                : '0';
              return (
                <li
                  key={invoiceId}
                  className={isSelected ? 'accordion-item open' : 'accordion-item'}
                >
                  <button
                    type="button"
                    className="accordion-trigger"
                    onClick={() => handleInvoiceClick(invoiceId)}
                    disabled={isLoading || isDetailLoading || seed.isDeriving || !seed.seedHex}
                  >
                    <div className="accordion-summary">
                      <code className="mono">{invoiceId}</code>
                      <span className="badge">
                        {isSingleInvoice(invoiceId) ? 'Single' : 'Batch'}
                      </span>
                    </div>
                    <span className="accordion-icon" aria-hidden="true">
                      {isSelected ? '−' : '+'}
                    </span>
                  </button>
                  {isSelected && (
                    <div className="accordion-body">
                      {invoiceDetail ? (
                        <RedeemDetailSection
                          title="Invoice Detail"
                          metadata={[
                            { label: 'Owner', value: <code className="mono">{invoiceDetail.owner}</code> },
                            { label: 'Chain', value: invoiceDetail.chainId.toString() },
                            { label: 'Type', value: invoiceDetail.isBatch ? 'Batch' : 'Single' },
                            {
                              label: 'Eligible Value Total',
                              value: <code className="mono">{eligibleTotal}</code>,
                            },
                            {
                              label: 'Pending Total Value',
                              value: <code className="mono">{pendingTotal}</code>,
                            },
                          ]}
                          items={invoiceDetail.burns.map((burnDetail) => {
                            const alreadyRedeemed =
                              burnDetail.context.totalTeleported >= burnDetail.context.totalIndexedValue;
                            const hasEligibleValue = burnDetail.context.totalEligibleValue > 0n;
                            const noRedeemableTransfers =
                              burnDetail.context.totalEligibleValue === 0n &&
                              burnDetail.context.totalTeleported === 0n;
                            return {
                              id: burnDetail.subId.toString(),
                              title: `Burn #${burnDetail.subId}`,
                              burnAddress: burnDetail.burn.burnAddress,
                              secret: burnDetail.burn.secret,
                              tweak: burnDetail.burn.tweak,
                              eligibleCount: burnDetail.context.events.eligible.length,
                              totalEvents:
                                burnDetail.context.events.ineligible.length +
                                burnDetail.context.events.eligible.length,
                              onRedeem: () => redeemBurn(burnDetail),
                              redeemDisabled: isLoading || isDetailLoading || alreadyRedeemed || !hasEligibleValue,
                              redeemedLabel: noRedeemableTransfers
                                ? 'No Redeemable Transfers'
                                : alreadyRedeemed
                                  ? 'Already redeemed'
                                  : undefined,
                              eligibleValue: formatEligibleValue(burnDetail.context.totalEligibleValue),
                              pendingValue: formatEligibleValue(burnDetail.context.totalPendingValue),
                            };
                          })}
                          message={redeemMessage}
                        />
                      ) : (
                        <p className="hint">
                          {isDetailLoading ? 'Loading invoice detail…' : 'Preparing invoice detail…'}
                        </p>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      <RedeemProgressModal
        open={isRedeemModalOpen}
        steps={redeemSteps}
        onClose={() => setRedeemModalOpen(false)}
        message={redeemMessage}
      />
    </section>
  );
}
