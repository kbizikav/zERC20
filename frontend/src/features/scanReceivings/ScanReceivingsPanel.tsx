import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import {
  BurnArtifacts,
  RedeemContext,
  ScannedAnnouncement,
  createAuthorizationPayload,
  requestVetKey,
  scanReceivings,
  decodeFullBurnAddress,
  findTokenByChain,
  createProviderForToken,
  getVerifierContract,
  collectRedeemContext,
  generateSingleTeleportProof,
  generateBatchTeleportProof,
  normalizeHex,
  getStealthClientFromConfig,
  getDeciderClient,
} from '@zerc20/sdk';
import { formatUnits, getBytes, hexlify, zeroPadValue } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens, TeleportWasmArtifacts } from '@zerc20/sdk';
import { useWallet } from '@app/providers/WalletProvider';
import { VetKey } from '@dfinity/vetkeys';
import { StealthError } from '@zerc20/sdk';
import { RedeemDetailSection } from '@features/redeem/RedeemDetailSection';
import { RedeemProgressModal } from '@features/redeem/RedeemProgressModal';
import { createRedeemSteps, setStepStatus, type RedeemStage, type RedeemStep } from '@features/redeem/redeemSteps';
import { yieldToUi } from '@features/redeem/yieldToUi';

interface ScanReceivingsPanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
  artifacts: TeleportWasmArtifacts;
  storageRevision: number;
}

interface AnnouncementDetail {
  id: string;
  burn: BurnArtifacts;
  context: RedeemContext;
  announcement: ScannedAnnouncement;
}

function padRecipient(address: string): string {
  return zeroPadValue(normalizeHex(address), 32);
}

function formatEligibleValue(value: bigint): string {
  return formatUnits(value, 18);
}

function formatAnnouncementTimestamp(ns: bigint): string {
  try {
    const ms = Number(ns / 1_000_000n);
    if (!Number.isFinite(ms)) {
      return 'Unknown';
    }
    return new Date(ms).toLocaleString();
  } catch {
    return 'Unknown';
  }
}

const VET_KEY_STORAGE_PREFIX = 'zerc20:vetkey:';

function vetKeyStorageKey(address: string): string {
  return `${VET_KEY_STORAGE_PREFIX}${address}`;
}

function loadStoredVetKey(address: string): VetKey | undefined {
  if (typeof window === 'undefined') {
    return undefined;
  }
  try {
    const stored = window.localStorage.getItem(vetKeyStorageKey(address));
    if (!stored) {
      return undefined;
    }
    const bytes = getBytes(stored);
    return VetKey.deserialize(bytes);
  } catch {
    window.localStorage.removeItem(vetKeyStorageKey(address));
    return undefined;
  }
}

function persistVetKey(address: string, vetKey: VetKey): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    const serialized = vetKey.serialize();
    const hex = hexlify(serialized);
    window.localStorage.setItem(vetKeyStorageKey(address), hex);
  } catch {
    // ignore storage write failures
  }
}

function removeStoredVetKey(address: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(vetKeyStorageKey(address));
  } catch {
    // ignore storage failures
  }
}

const ANNOUNCEMENT_STORAGE_PREFIX = 'zerc20:announcements:';

interface StoredAnnouncement {
  id: string;
  burnAddress: string;
  fullBurnAddress: string;
  createdAtNs: string;
  recipientChainId: string;
}

function announcementStorageKey(address: string): string {
  return `${ANNOUNCEMENT_STORAGE_PREFIX}${normalizeHex(address)}`;
}

function toStoredAnnouncement(announcement: ScannedAnnouncement): StoredAnnouncement {
  return {
    id: announcement.id.toString(),
    burnAddress: announcement.burnAddress,
    fullBurnAddress: announcement.fullBurnAddress,
    createdAtNs: announcement.createdAtNs.toString(),
    recipientChainId: announcement.recipientChainId.toString(),
  };
}

function loadStoredAnnouncements(address: string): ScannedAnnouncement[] {
  if (typeof window === 'undefined') {
    return [];
  }
  const key = announcementStorageKey(address);
  try {
    const stored = window.localStorage.getItem(key);
    if (!stored) {
      return [];
    }
    const parsed: unknown = JSON.parse(stored);
    if (!Array.isArray(parsed)) {
      window.localStorage.removeItem(key);
      return [];
    }
    const entries = new Map<string, ScannedAnnouncement>();
    for (const item of parsed) {
      if (!item || typeof item !== 'object') {
        continue;
      }
      const candidate = item as Partial<StoredAnnouncement>;
      if (
        typeof candidate.id !== 'string' ||
        typeof candidate.burnAddress !== 'string' ||
        typeof candidate.fullBurnAddress !== 'string' ||
        typeof candidate.createdAtNs !== 'string' ||
        typeof candidate.recipientChainId !== 'string'
      ) {
        continue;
      }
      try {
        const announcement: ScannedAnnouncement = {
          id: BigInt(candidate.id),
          burnAddress: candidate.burnAddress,
          fullBurnAddress: candidate.fullBurnAddress,
          createdAtNs: BigInt(candidate.createdAtNs),
          recipientChainId: BigInt(candidate.recipientChainId),
        };
        entries.set(announcement.id.toString(), announcement);
      } catch {
        continue;
      }
    }
    const restored = Array.from(entries.values()).sort((a, b) => {
      if (a.createdAtNs === b.createdAtNs) {
        if (a.id === b.id) {
          return 0;
        }
        return a.id > b.id ? -1 : 1;
      }
      return a.createdAtNs > b.createdAtNs ? -1 : 1;
    });
    return restored;
  } catch {
    window.localStorage.removeItem(key);
    return [];
  }
}

function persistAnnouncements(address: string, announcements: ScannedAnnouncement[]): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    const serialized = announcements.map(toStoredAnnouncement);
    window.localStorage.setItem(announcementStorageKey(address), JSON.stringify(serialized));
  } catch {
    // ignore storage write failures
  }
}

export function ScanReceivingsPanel({
  config,
  tokens,
  artifacts,
  storageRevision,
}: ScanReceivingsPanelProps): JSX.Element {
  const wallet = useWallet();
  const [announcements, setAnnouncements] = useState<ScannedAnnouncement[]>([]);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const [isScanning, setIsScanning] = useState(false);
  const [isDetailLoading, setIsDetailLoading] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [redeemMessage, setRedeemMessage] = useState<string>();
  const [cachedVetKey, setCachedVetKey] = useState<VetKey>();
  const [selectedAnnouncementId, setSelectedAnnouncementId] = useState<string>();
  const [detail, setDetail] = useState<AnnouncementDetail | null>(null);
  const [redeemSteps, setRedeemSteps] = useState<RedeemStep[]>([]);
  const [isRedeemModalOpen, setRedeemModalOpen] = useState(false);

  useEffect(() => {
    if (!wallet.account) {
      setCachedVetKey(undefined);
      return;
    }
    const stored = loadStoredVetKey(wallet.account);
    setCachedVetKey(stored);
  }, [wallet.account, storageRevision]);

  useEffect(() => {
    if (!wallet.account) {
      setAnnouncements([]);
      setSelectedAnnouncementId(undefined);
      setDetail(null);
      setStatus(undefined);
      setError(undefined);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);
      setIsScanning(false);
      setIsDetailLoading(false);
      setIsLoading(false);
      return;
    }
    try {
      const stored = loadStoredAnnouncements(wallet.account);
      setAnnouncements(stored);
    } catch {
      setAnnouncements([]);
    }
    setSelectedAnnouncementId(undefined);
    setDetail(null);
    setStatus(undefined);
    setError(undefined);
    setRedeemMessage(undefined);
    setRedeemSteps([]);
    setRedeemModalOpen(false);
    setIsScanning(false);
    setIsDetailLoading(false);
    setIsLoading(false);
  }, [wallet.account, storageRevision]);

  const availableTokens = useMemo(() => tokens.tokens ?? [], [tokens.tokens]);

  const mergeAnnouncements = useCallback(
    (accountAddress: string, scanned: ScannedAnnouncement[]): { added: number; total: number } => {
      const normalizedAddress = normalizeHex(accountAddress);
      let added = 0;
      let total = 0;
      setAnnouncements((prev) => {
        const entries = new Map(prev.map((item) => [item.id.toString(), item]));
        for (const item of scanned) {
          const key = item.id.toString();
          if (!entries.has(key)) {
            entries.set(key, item);
            added += 1;
          }
        }
        const next = Array.from(entries.values()).sort((a, b) => {
          if (a.createdAtNs === b.createdAtNs) {
            if (a.id === b.id) {
              return 0;
            }
            return a.id > b.id ? -1 : 1;
          }
          return a.createdAtNs > b.createdAtNs ? -1 : 1;
        });
        total = next.length;
        persistAnnouncements(normalizedAddress, next);
        return next;
      });
      return { added, total };
    },
    [],
  );

  const filteredAnnouncements = useMemo(() => {
    if (wallet.chainId === undefined || wallet.chainId === null) {
      return announcements;
    }
    try {
      const chain = BigInt(wallet.chainId);
      return announcements.filter((item) => item.recipientChainId === chain);
    } catch {
      return announcements;
    }
  }, [announcements, wallet.chainId]);

  useEffect(() => {
    if (!selectedAnnouncementId) {
      return;
    }
    const stillVisible = filteredAnnouncements.some(
      (item) => item.id.toString() === selectedAnnouncementId,
    );
    if (!stillVisible) {
      setSelectedAnnouncementId(undefined);
      setDetail(null);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);
    }
  }, [filteredAnnouncements, selectedAnnouncementId]);

  const handleScan = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setStatus(undefined);
      setError(undefined);
      setRedeemMessage(undefined);
      setDetail(null);
      setSelectedAnnouncementId(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);

      if (!wallet.account) {
        setError('Connect your wallet before scanning announcements.');
        return;
      }
      if (!tokens.hub) {
        setError('Tokens and hub configurations are required.');
        return;
      }

      const accountAddress = wallet.account;
      let normalizedAccount: string;
      try {
        normalizedAccount = normalizeHex(accountAddress);
      } catch {
        setError('Invalid wallet address. Reconnect your wallet and try again.');
        return;
      }

      const performScan = async (forceRenew: boolean): Promise<ScannedAnnouncement[]> => {
        const client = await getStealthClientFromConfig({
          icReplicaUrl: config.icReplicaUrl,
          storageCanisterId: config.storageCanisterId,
          keyManagerCanisterId: config.keyManagerCanisterId,
        });
        let vetKeyToUse = forceRenew ? undefined : cachedVetKey;

        if (forceRenew) {
          removeStoredVetKey(accountAddress);
          setCachedVetKey(undefined);
        }

        if (!vetKeyToUse) {
          setStatus(forceRenew ? 'Refreshing authorization…' : 'Preparing authorization…');
          const payload = await createAuthorizationPayload(client, accountAddress, config.authorizationTtlSeconds);
          const signer = await wallet.ensureSigner();
          const signatureHex = await signer.signMessage(payload.message);
          const signatureBytes = getBytes(signatureHex);

          setStatus(forceRenew ? 'Requesting new view key…' : 'Requesting view key…');
          const requestedVetKey = await requestVetKey(client, accountAddress, payload, signatureBytes);
          persistVetKey(accountAddress, requestedVetKey);
          setCachedVetKey(requestedVetKey);
          vetKeyToUse = requestedVetKey;
        } else {
          setStatus('Using saved view key…');
        }

        if (!vetKeyToUse) {
          throw new Error('Unable to acquire view key for scanning.');
        }

        setStatus('Scanning announcements…');
        const scanned = await scanReceivings({
          client,
          vetKey: vetKeyToUse,
          pageSize: config.scanPageSize,
        });

        return scanned;
      };

      try {
        setIsScanning(true);
        const scanned = await performScan(false);
        const { added, total } = mergeAnnouncements(accountAddress, scanned);
        setError(undefined);
        if (total === 0) {
          setStatus('No new entries found.');
        } else if (added === 0) {
          setStatus(`No new announcements. Stored ${total} announcement(s) for ${normalizedAccount}.`);
        } else {
          setStatus(`Added ${added} new announcement(s). Stored ${total} announcement(s) for ${normalizedAccount}.`);
        }
      } catch (err) {
        if (err instanceof StealthError && cachedVetKey) {
          try {
            const rescanned = await performScan(true);
            const { added, total } = mergeAnnouncements(accountAddress, rescanned);
            setError(undefined);
            if (total === 0) {
              setStatus('No new entries found.');
            } else if (added === 0) {
              setStatus(`No new announcements. Stored ${total} announcement(s) for ${normalizedAccount}.`);
            } else {
              setStatus(
                `Added ${added} new announcement(s). Stored ${total} announcement(s) for ${normalizedAccount}.`,
              );
            }
            return;
          } catch (retryError) {
            const message = retryError instanceof Error ? retryError.message : String(retryError);
            setError(message);
            setStatus(undefined);
            return;
          }
        }
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsScanning(false);
      }
    },
    [wallet, tokens, config, cachedVetKey, mergeAnnouncements],
  );

  const handleAnnouncementDetail = useCallback(
    async (announcement: ScannedAnnouncement) => {
      const announcementId = announcement.id.toString();
      if (selectedAnnouncementId === announcementId) {
        setSelectedAnnouncementId(undefined);
        setRedeemMessage(undefined);
        setRedeemSteps([]);
        setRedeemModalOpen(false);
        return;
      }

      if (!tokens.hub) {
        setError('Tokens and hub configurations are required.');
        return;
      }

      setSelectedAnnouncementId(announcementId);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);
      setError(undefined);

      if (detail && detail.id === announcementId) {
        return;
      }

      setDetail(null);

      try {
        setIsDetailLoading(true);
        setStatus('Loading announcement detail…');
        const burn = await decodeFullBurnAddress(announcement.fullBurnAddress);
        const token = findTokenByChain(availableTokens, burn.generalRecipient.chainId);

        const tokenProvider = createProviderForToken(token);
        const verifierContract = getVerifierContract(token.verifierAddress, tokenProvider);

        const context = await collectRedeemContext({
          burn,
          tokens: availableTokens,
          hub: tokens.hub,
          verifierContract,
          indexerUrl: config.indexerUrl,
          indexerFetchLimit: config.indexerFetchLimit,
          eventBlockSpan: BigInt(config.eventBlockSpan),
        });

        setDetail({
          id: announcementId,
          burn,
          context,
          announcement,
        });
        await yieldToUi();
        setStatus('Announcement detail loaded.');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsDetailLoading(false);
      }
    },
    [tokens, availableTokens, config, selectedAnnouncementId, detail],
  );

  const redeemAnnouncement = useCallback(
    async () => {
      if (!detail) {
        setRedeemMessage('Load an announcement detail before redeeming.');
        return;
      }
      if (!wallet.signer) {
        setRedeemMessage('Connect your wallet before redeeming transfers.');
        return;
      }
      if (!tokens.hub) {
        setRedeemMessage('Tokens and hub configurations are required.');
        return;
      }

      const initialSteps = createRedeemSteps(detail.context.events.eligible.length > 1);
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

        const tokenEntry = detail.context.token;
        const tokenProvider = createProviderForToken(tokenEntry);
        const verifierRead = getVerifierContract(tokenEntry.verifierAddress, tokenProvider);

        const refreshedContext = await collectRedeemContext({
          burn: detail.burn,
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
          return { ...current, context: refreshedContext };
        });
        await yieldToUi();

        if (refreshedContext.totalTeleported >= refreshedContext.totalIndexedValue) {
          setRedeemMessage('This announcement has already been redeemed.');
          setRedeemModalOpen(false);
          await yieldToUi();
          return;
        }

        const eligible = refreshedContext.events.eligible;
        if (eligible.length === 0) {
          setRedeemMessage('No eligible transfers found for this announcement.');
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
          chainId: detail.burn.generalRecipient.chainId,
          recipient: padRecipient(detail.burn.generalRecipient.address),
          tweak: detail.burn.generalRecipient.tweak,
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
            recipientFr: detail.burn.generalRecipient.fr,
            secretHex: detail.burn.secret,
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
          setRedeemMessage(`Teleport submitted: ${tx.hash}`);
        } else {
          const fields = artifacts.batch;
          if (!fields.localPp || !fields.localVp || !fields.globalPp || !fields.globalVp) {
            throw new Error('Upload all batch teleport Nova artifacts before redeeming.');
          }
          const decider = getDeciderClient({ baseUrl: config.deciderUrl });
          const batchProof = await generateBatchTeleportProof({
            wasmArtifacts: {
              localPp: fields.localPp,
              localVp: fields.localVp,
              globalPp: fields.globalPp,
              globalVp: fields.globalVp,
            },
            aggregationState: refreshedContext.aggregationState,
            recipientFr: detail.burn.generalRecipient.fr,
            secretHex: detail.burn.secret,
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
    [detail, wallet.signer, tokens, availableTokens, artifacts, config],
  );

  return (
    <section className="card">
      <header className="card-header">
        <h2>Receivings</h2>
      </header>
      {!tokens.hub && (
        <div className="card-body">
          <p className="error">Hub configuration is missing from tokens.json.</p>
        </div>
      )}
      {tokens.hub && (
        <form className="card-body" onSubmit={handleScan}>
          <footer className="card-footer full card-footer-spread">
            <div className="card-footer-status">
              {status && <span>{status}</span>}
              {error && <span className="error">{error}</span>}
            </div>
            <div className="card-footer-actions">
              <button
                type="submit"
                className="primary"
                disabled={isScanning || isDetailLoading || isLoading}
              >
                {isScanning ? 'Scanning…' : 'Scan'}
              </button>
            </div>
          </footer>
        </form>
      )}

      {filteredAnnouncements.length > 0 && (
        <div className="card-section">
          <ul className="list accordion">
            {filteredAnnouncements.map((item) => {
              const announcementId = item.id.toString();
              const isSelected = announcementId === selectedAnnouncementId;
              const announcementDetail =
                detail && detail.id === announcementId ? detail : null;
              const eligibleValueDisplay = announcementDetail
                ? formatEligibleValue(announcementDetail.context.totalEligibleValue)
                : '0';
              const pendingValueDisplay = announcementDetail
                ? formatEligibleValue(announcementDetail.context.totalPendingValue)
                : '0';
              const isAnnouncementRedeemed =
                announcementDetail !== null
                  ? announcementDetail.context.totalTeleported >= announcementDetail.context.totalIndexedValue
                  : false;
              const hasEligibleValue =
                announcementDetail !== null ? announcementDetail.context.totalEligibleValue > 0n : false;
              const noRedeemableTransfers =
                announcementDetail !== null
                  ? announcementDetail.context.totalEligibleValue === 0n &&
                    announcementDetail.context.totalTeleported === 0n
                  : false;
              return (
                <li
                  key={announcementId}
                  className={isSelected ? 'accordion-item open' : 'accordion-item'}
                >
                  <button
                    type="button"
                    className="accordion-trigger"
                    onClick={() => handleAnnouncementDetail(item)}
                    disabled={isScanning || isDetailLoading || isLoading}
                  >
                    <div className="accordion-summary">
                      <div className="accordion-title-group">
                        <span className="accordion-title">ID {announcementId}</span>
                        <code className="mono">{item.burnAddress}</code>
                      </div>
                      <span className="timestamp">
                        {formatAnnouncementTimestamp(item.createdAtNs)}
                      </span>
                    </div>
                    <span className="accordion-icon" aria-hidden="true">
                      {isSelected ? '−' : '+'}
                    </span>
                  </button>
                  {isSelected && (
                    <div className="accordion-body">
                      {announcementDetail ? (
                        <RedeemDetailSection
                          title="Announcement Detail"
                          metadata={[
                            { label: 'Announcement ID', value: announcementId },
                            {
                              label: 'Chain',
                              value: announcementDetail.burn.generalRecipient.chainId.toString(),
                            },
                            {
                              label: 'Type',
                              value:
                                announcementDetail.context.events.eligible.length > 1 ? 'Batch' : 'Single',
                            },
                            {
                              label: 'Eligible Value Total',
                              value: <code className="mono">{eligibleValueDisplay}</code>,
                            },
                            {
                              label: 'Pending Total Value',
                              value: <code className="mono">{pendingValueDisplay}</code>,
                            },
                          ]}
                          items={[
                            {
                              id: announcementDetail.id,
                              title: 'Announcement',
                              burnAddress: announcementDetail.burn.burnAddress,
                              secret: announcementDetail.burn.secret,
                              tweak: announcementDetail.burn.tweak,
                              eligibleCount: announcementDetail.context.events.eligible.length,
                              totalEvents:
                                announcementDetail.context.events.eligible.length +
                                announcementDetail.context.events.ineligible.length,
                              onRedeem: redeemAnnouncement,
                              redeemDisabled:
                                isScanning || isDetailLoading || isLoading || isAnnouncementRedeemed || !hasEligibleValue,
                              redeemedLabel: noRedeemableTransfers
                                ? 'No Redeemable Transfers'
                                : isAnnouncementRedeemed
                                  ? 'Already redeemed'
                                  : undefined,
                              eligibleValue: eligibleValueDisplay,
                              pendingValue: pendingValueDisplay,
                            },
                          ]}
                          message={redeemMessage}
                        />
                      ) : (
                        <p className="hint">
                          {isDetailLoading ? 'Loading announcement detail…' : 'Preparing announcement detail…'}
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
      {filteredAnnouncements.length === 0 && announcements.length > 0 && (
        <div className="card-section">
          <p className="hint">
            No announcements match the connected chain. Switch networks to view stored announcements.
          </p>
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
