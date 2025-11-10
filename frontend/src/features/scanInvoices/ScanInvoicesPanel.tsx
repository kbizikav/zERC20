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
  generateBatchTeleportProof,
  generateSingleTeleportProof,
} from '@zerc20/sdk';
import { formatUnits, getBytes, zeroPadValue } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens, TeleportArtifacts } from '@/types/app';
import { useWallet } from '@app/providers/WalletProvider';
import { useSeed } from '@/hooks/useSeed';
import { createDeciderClient, getStealthClient } from '@services/clients';
import { RedeemDetailSection } from '@features/redeem/RedeemDetailSection';
import { RedeemProgressModal } from '@features/redeem/RedeemProgressModal';
import { createRedeemSteps, setStepStatus, type RedeemStage, type RedeemStep } from '@features/redeem/redeemSteps';
import { yieldToUi } from '@features/redeem/yieldToUi';

interface ScanInvoicesPanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
  artifacts: TeleportArtifacts;
  storageRevision: number;
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

const INVOICE_STORAGE_PREFIX = 'zerc20:invoices:';

function invoiceStorageKey(address: string): string {
  return `${INVOICE_STORAGE_PREFIX}${normalizeHex(address)}`;
}

function loadStoredInvoices(address: string): string[] {
  if (typeof window === 'undefined') {
    return [];
  }
  try {
    const stored = window.localStorage.getItem(invoiceStorageKey(address));
    if (!stored) {
      return [];
    }
    const parsed: unknown = JSON.parse(stored);
    if (!Array.isArray(parsed)) {
      window.localStorage.removeItem(invoiceStorageKey(address));
      return [];
    }
    const normalized = parsed
      .filter((value): value is string => typeof value === 'string')
      .map((value) => {
        try {
          return normalizeHex(value);
        } catch {
          return null;
        }
      })
      .filter((value): value is string => value !== null);
    return Array.from(new Set(normalized));
  } catch {
    window.localStorage.removeItem(invoiceStorageKey(address));
    return [];
  }
}

function persistInvoices(address: string, invoiceIds: string[]): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(invoiceStorageKey(address), JSON.stringify(invoiceIds));
  } catch {
    // ignore storage write failures
  }
}

export function ScanInvoicesPanel({
  config,
  tokens,
  artifacts,
  storageRevision,
}: ScanInvoicesPanelProps): JSX.Element {
  const wallet = useWallet();
  const seed = useSeed();
  const [invoices, setInvoices] = useState<string[]>([]);
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
  useEffect(() => {
    if (!wallet.account || !connectedToken) {
      setInvoices([]);
      setSelectedInvoice(undefined);
      setDetail(null);
      setIsDetailLoading(false);
      return;
    }
    try {
      const stored = loadStoredInvoices(wallet.account);
      const filtered = stored.filter((invoiceId) => {
        try {
          return extractChainIdFromInvoiceHex(invoiceId) === connectedToken.chainId;
        } catch {
          return false;
        }
      });
      setInvoices(filtered);
    } catch {
      setInvoices([]);
    }
    setSelectedInvoice(undefined);
    setDetail(null);
    setIsDetailLoading(false);
  }, [wallet.account, connectedToken, storageRevision]);

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
        const stealthClient = await getStealthClient(config);
        const ids = await listInvoices(stealthClient, owner, connectedToken.chainId);
        const normalizedOwner = owner;
        const normalizedIds = ids.map((value) => normalizeHex(value));
        let added = 0;
        let nextLength = 0;
        setInvoices((prev) => {
          const filteredPrev = prev.filter((invoiceId) => {
            try {
              return extractChainIdFromInvoiceHex(invoiceId) === connectedToken.chainId;
            } catch {
              return false;
            }
          });
          const existing = new Set(filteredPrev);
          const next = [...filteredPrev];
          for (const id of normalizedIds) {
            if (!existing.has(id)) {
              existing.add(id);
              next.push(id);
              added += 1;
            }
          }
          nextLength = next.length;
          persistInvoices(normalizedOwner, next);
          return next;
        });
        if (nextLength === 0) {
          setStatus('No new entries found.');
        } else if (added === 0) {
          setStatus(`No new invoices. Stored ${nextLength} invoice(s) for ${owner}.`);
        } else {
          setStatus(`Added ${added} new invoice(s). Stored ${nextLength} invoice(s) for ${owner}.`);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsLoading(false);
      }
    },
    [wallet, connectedToken, config],
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
        const verifierContract = getVerifierContract(connectedToken.verifierAddress, tokenProvider);

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
      if (!wallet.signer) {
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
        const verifierRead = getVerifierContract(tokenEntry.verifierAddress, tokenProvider);

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

        const verifierWithSigner = getVerifierContract(refreshedContext.token.verifierAddress, wallet.signer);
        const gr = {
          chainId: burnDetail.burn.generalRecipient.chainId,
          recipient: padRecipient(burnDetail.burn.generalRecipient.address),
          tweak: burnDetail.burn.generalRecipient.tweak,
        };

        activateStage('proof', 'Generating WASM proof…');
        await yieldToUi();

        if (eligible.length === 1) {
          const fields = artifacts.single;
          if (!fields.localPk || !fields.localVk || !fields.globalPk || !fields.globalVk) {
            throw new Error('Upload all single teleport Groth16 artifacts before redeeming.');
          }
          const singleProof = await generateSingleTeleportProof({
            wasmArtifacts: {
              localPk: fields.localPk,
              localVk: fields.localVk,
              globalPk: fields.globalPk,
              globalVk: fields.globalVk,
            },
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

          const tx = await verifierWithSigner.singleTeleport(
            true,
            refreshedContext.aggregationState.latestAggSeq,
            gr,
            getBytes(singleProof.proofCalldata),
          );
          await tx.wait();
          completeStage('wallet');
          await yieldToUi();
          setRedeemMessage(`Single teleport submitted: ${tx.hash}`);
        } else {
          const fields = artifacts.batch;
          if (!fields.localPp || !fields.localVp || !fields.globalPp || !fields.globalVp) {
            throw new Error('Upload all batch teleport Nova artifacts before redeeming.');
          }
          const decider = createDeciderClient(config);
          const batchProof = await generateBatchTeleportProof({
            wasmArtifacts: {
              localPp: fields.localPp,
              localVp: fields.localVp,
              globalPp: fields.globalPp,
              globalVp: fields.globalVp,
            },
            aggregationState: refreshedContext.aggregationState,
            recipientFr: burnDetail.burn.generalRecipient.fr,
            secretHex: burnDetail.burn.secret,
            events: eligible,
            proofs: refreshedContext.globalProofs,
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

          const tx = await verifierWithSigner.teleport(
            true,
            refreshedContext.aggregationState.latestAggSeq,
            gr,
            batchProof.deciderProof,
          );
          await tx.wait();
          completeStage('wallet');
          await yieldToUi();
          setRedeemMessage(`Batch teleport submitted: ${tx.hash}`);
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
    [detail, wallet.signer, tokens, artifacts, config, availableTokens],
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
