import { CSSProperties, FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Key, ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
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
  isSingleInvoiceHex,
  getHubContract,
  getVerifierContract,
  createProviderForHub,
  createProviderForToken,
  generateBatchTeleportProof,
  generateSingleTeleportProof,
} from '@services/sdk';
import { formatUnits, getBytes, zeroPadValue } from 'ethers';
import type { AppConfig } from '@config/appConfig';
import type { NormalizedTokens, TeleportArtifacts } from '@/types/app';
import { useWallet } from '@app/providers/WalletProvider';
import { useSeed } from '@/hooks/useSeed';
import { createDeciderClient, createIndexerClient, getStealthClient } from '@services/clients';
import { RedeemDetailSection, RedeemDetailChainSummary } from '@features/redeem/RedeemDetailSection';
import { RedeemProgressModal } from '@features/redeem/RedeemProgressModal';
import { createRedeemSteps, setStepStatus, type RedeemStage, type RedeemStep } from '@features/redeem/redeemSteps';
import { createTeleportSubmittedMessage } from '@features/redeem/redeemMessages';
import { yieldToUi } from '@features/redeem/yieldToUi';

interface ScanInvoicesPanelProps {
  config: AppConfig;
  tokens: NormalizedTokens;
  artifacts: TeleportArtifacts;
  storageRevision: number;
  reloadRevision?: number;
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

function padRecipient(address: string): string {
  return zeroPadValue(normalizeHex(address), 32);
}

function formatEligibleValue(value: bigint): string {
  return formatUnits(value, 18);
}

function createChainSummaries(chains: RedeemContext['chains']): RedeemDetailChainSummary[] {
  return chains.reduce<RedeemDetailChainSummary[]>((acc, chain) => {
    const hasActivity =
      chain.events.eligible.length > 0 || chain.events.ineligible.length > 0;
    if (!hasActivity) {
      return acc;
    }
    const chainId = chain.chainId.toString();
    const summary: RedeemDetailChainSummary = {
      chainId,
      name: chain.token.label ?? `Chain ${chainId}`,
      eligibleValue: formatEligibleValue(chain.totalEligibleValue),
      pendingValue: formatEligibleValue(chain.totalPendingValue),
      eligibleEvents: chain.events.eligible.map((event) => ({
        eventIndex: event.eventIndex.toString(),
        from: event.from,
        to: event.to,
        value: formatEligibleValue(event.value),
      })),
      pendingEvents: chain.events.ineligible.map((event) => ({
        eventIndex: event.eventIndex.toString(),
        from: event.from,
        to: event.to,
        value: formatEligibleValue(event.value),
      })),
    };
    acc.push(summary);
    return acc;
  }, []);
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
    const unique = Array.from(new Set(normalized));
    return sortInvoiceIds(unique);
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

function sortInvoiceIds(invoiceIds: string[]): string[] {
  return [...invoiceIds].sort((a, b) => {
    const aValue = BigInt(a);
    const bValue = BigInt(b);
    if (aValue === bValue) {
      return 0;
    }
    return aValue > bValue ? -1 : 1;
  });
}

const BURN_STORAGE_PREFIX = 'zerc20:burns:';

interface StoredBurnArtifact {
  burnAddress: string;
  fullBurnAddress: string;
  secret: string;
  tweak: string;
  generalRecipient: {
    chainId: string;
    address: string;
    tweak: string;
    fr: string;
    u256: string;
  };
}

type StoredBurnStore = Record<string, Record<string, StoredBurnArtifact>>;

function burnStorageKey(address: string): string {
  return `${BURN_STORAGE_PREFIX}${normalizeHex(address)}`;
}

function coerceStoredBurnArtifact(value: unknown): StoredBurnArtifact | null {
  if (!value || typeof value !== 'object') {
    return null;
  }
  const entry = value as Record<string, unknown>;
  const general = entry.generalRecipient as Record<string, unknown> | undefined;
  if (!general || typeof general !== 'object') {
    return null;
  }

  const burnAddress = entry.burnAddress;
  const fullBurnAddress = entry.fullBurnAddress;
  const secret = entry.secret;
  const tweak = entry.tweak;
  const chainId = general.chainId;
  const address = general.address;
  const generalTweak = general.tweak;
  const fr = general.fr;
  const u256 = general.u256;

  if (
    typeof burnAddress !== 'string' ||
    typeof fullBurnAddress !== 'string' ||
    typeof secret !== 'string' ||
    typeof tweak !== 'string' ||
    typeof chainId !== 'string' ||
    typeof address !== 'string' ||
    typeof generalTweak !== 'string' ||
    typeof fr !== 'string' ||
    typeof u256 !== 'string'
  ) {
    return null;
  }

  return {
    burnAddress,
    fullBurnAddress,
    secret,
    tweak,
    generalRecipient: {
      chainId,
      address,
      tweak: generalTweak,
      fr,
      u256,
    },
  };
}

function loadStoredBurnArtifacts(address: string): StoredBurnStore {
  if (typeof window === 'undefined') {
    return {};
  }
  try {
    const stored = window.localStorage.getItem(burnStorageKey(address));
    if (!stored) {
      return {};
    }
    const parsed: unknown = JSON.parse(stored);
    if (!parsed || typeof parsed !== 'object') {
      window.localStorage.removeItem(burnStorageKey(address));
      return {};
    }

    const result: StoredBurnStore = {};
    for (const [invoiceId, subMap] of Object.entries(parsed as Record<string, unknown>)) {
      if (!subMap || typeof subMap !== 'object') {
        continue;
      }
      const entries: Record<string, StoredBurnArtifact> = {};
      for (const [subId, burnValue] of Object.entries(subMap as Record<string, unknown>)) {
        const coerced = coerceStoredBurnArtifact(burnValue);
        if (coerced) {
          entries[subId] = coerced;
        }
      }
      if (Object.keys(entries).length > 0) {
        result[invoiceId] = entries;
      }
    }
    return result;
  } catch {
    window.localStorage.removeItem(burnStorageKey(address));
    return {};
  }
}

function persistStoredBurnArtifacts(address: string, store: StoredBurnStore): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(burnStorageKey(address), JSON.stringify(store));
  } catch {
    // ignore storage write failures
  }
}

function serializeBurnArtifact(burn: BurnArtifacts): StoredBurnArtifact {
  return {
    burnAddress: burn.burnAddress,
    fullBurnAddress: burn.fullBurnAddress,
    secret: burn.secret,
    tweak: burn.tweak,
    generalRecipient: {
      chainId: burn.generalRecipient.chainId.toString(),
      address: burn.generalRecipient.address,
      tweak: burn.generalRecipient.tweak,
      fr: burn.generalRecipient.fr,
      u256: burn.generalRecipient.u256,
    },
  };
}

function deserializeBurnArtifact(stored: StoredBurnArtifact): BurnArtifacts {
  return {
    burnAddress: normalizeHex(stored.burnAddress),
    fullBurnAddress: normalizeHex(stored.fullBurnAddress),
    secret: normalizeHex(stored.secret),
    tweak: normalizeHex(stored.tweak),
    generalRecipient: {
      chainId: BigInt(stored.generalRecipient.chainId),
      address: normalizeHex(stored.generalRecipient.address),
      tweak: normalizeHex(stored.generalRecipient.tweak),
      fr: normalizeHex(stored.generalRecipient.fr),
      u256: normalizeHex(stored.generalRecipient.u256),
    },
  };
}

function getStoredBurnArtifact(
  store: StoredBurnStore,
  invoiceId: string,
  subId: number,
): BurnArtifacts | null {
  const invoiceStore = store[invoiceId];
  if (!invoiceStore) {
    return null;
  }
  const stored = invoiceStore[subId.toString()];
  return stored ? deserializeBurnArtifact(stored) : null;
}

function setStoredBurnArtifact(
  store: StoredBurnStore,
  invoiceId: string,
  subId: number,
  burn: BurnArtifacts,
): void {
  const key = subId.toString();
  if (!store[invoiceId]) {
    store[invoiceId] = {};
  }
  store[invoiceId][key] = serializeBurnArtifact(burn);
}

export function ScanInvoicesPanel({
  config,
  tokens,
  artifacts,
  storageRevision,
  reloadRevision = 0,
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
  const [redeemMessage, setRedeemMessage] = useState<ReactNode>();
  const [redeemSteps, setRedeemSteps] = useState<RedeemStep[]>([]);
  const [isRedeemModalOpen, setRedeemModalOpen] = useState(false);
  const [invoiceLabels, setInvoiceLabels] = useState<Record<string, string>>({});
  const reloadRevisionRef = useRef(reloadRevision);
  const listParentRef = useRef<HTMLDivElement | null>(null);

  const shouldVirtualize = invoices.length > 20;
  const estimateInvoiceSize = useCallback(() => 96, []);
  const invoiceVirtualizer = useVirtualizer({
    count: shouldVirtualize ? invoices.length : 0,
    getScrollElement: () => listParentRef.current,
    estimateSize: estimateInvoiceSize,
    overscan: 8,
  });

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
      setInvoiceLabels({});
      return;
    }
    try {
      const stored = loadStoredInvoices(wallet.account);
      const filtered = sortInvoiceIds(
        stored.filter((invoiceId) => {
          try {
            return extractChainIdFromInvoiceHex(invoiceId) === connectedToken.chainId;
          } catch {
            return false;
          }
        }),
      );
      setInvoices(filtered);
      setInvoiceLabels((prev) => {
        const next: Record<string, string> = {};
        for (const invoiceId of filtered) {
          const label = prev[invoiceId];
          if (label) {
            next[invoiceId] = label;
          }
        }
        return next;
      });
    } catch {
      setInvoices([]);
      setInvoiceLabels({});
    }
    setSelectedInvoice(undefined);
    setDetail(null);
    setIsDetailLoading(false);
  }, [wallet.account, connectedToken, storageRevision]);

  useEffect(() => {
    if (!wallet.account || !connectedToken) {
      return;
    }
    const missing = invoices.filter((invoiceId) => !invoiceLabels[invoiceId]);
    if (missing.length === 0) {
      return;
    }

    let cancelled = false;
    const owner = normalizeHex(wallet.account);
    const storedBurns = loadStoredBurnArtifacts(owner);

    const cachedEntries: Record<string, string> = {};
    const pendingInvoices: string[] = [];

    for (const invoiceId of missing) {
      const cachedBurn = getStoredBurnArtifact(storedBurns, invoiceId, 0);
      if (cachedBurn) {
        cachedEntries[invoiceId] = cachedBurn.burnAddress;
      } else {
        pendingInvoices.push(invoiceId);
      }
    }

    if (Object.keys(cachedEntries).length > 0) {
      setInvoiceLabels((prev) => ({ ...prev, ...cachedEntries }));
    }

    if (pendingInvoices.length === 0 || !seed.seedHex) {
      return () => {
        cancelled = true;
      };
    }

    const seedHex = seed.seedHex;
    const chainId = connectedToken.chainId;

    const populateLabels = async () => {
      const computedEntries: Record<string, string> = {};
      let storeMutated = false;

      for (const invoiceId of pendingInvoices) {
        try {
          const invoiceIsSingle = isSingleInvoiceHex(invoiceId);
          const secretAndTweak = invoiceIsSingle
            ? await deriveInvoiceSingle(seedHex, invoiceId, chainId, owner)
            : await deriveInvoiceBatch(seedHex, invoiceId, 0, chainId, owner);
          const burn = await buildFullBurnAddress(chainId, owner, secretAndTweak.secret, secretAndTweak.tweak);
          setStoredBurnArtifact(storedBurns, invoiceId, 0, burn);
          storeMutated = true;
          if (!cancelled) {
            computedEntries[invoiceId] = burn.burnAddress;
          }
        } catch {
          // ignore derivation errors; they will be surfaced during detail load
        }
      }

      if (cancelled) {
        return;
      }
      if (Object.keys(computedEntries).length > 0) {
        setInvoiceLabels((prev) => ({ ...prev, ...computedEntries }));
      }
      if (storeMutated) {
        persistStoredBurnArtifacts(owner, storedBurns);
      }
    };

    void populateLabels();
    return () => {
      cancelled = true;
    };
  }, [invoices, invoiceLabels, wallet.account, connectedToken, seed.seedHex]);

  const reloadInvoices = useCallback(async () => {
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
      if (!seed.seedHex) {
        setStatus('Requesting seed authorization…');
        try {
          await seed.deriveSeed();
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          setError(message);
          setStatus(undefined);
          return;
        }
      }
      setStatus('Reloading invoices…');
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
        const sortedNext = sortInvoiceIds(next);
        nextLength = sortedNext.length;
        persistInvoices(normalizedOwner, sortedNext);
        setInvoiceLabels((current) => {
          const trimmed: Record<string, string> = {};
          for (const invoiceId of sortedNext) {
            const label = current[invoiceId];
            if (label) {
              trimmed[invoiceId] = label;
            }
          }
          return trimmed;
        });
        return sortedNext;
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
  }, [wallet, connectedToken, config, seed]);

  const handleLoadInvoices = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      void reloadInvoices();
    },
    [reloadInvoices],
  );

  useEffect(() => {
    if (!reloadRevision || reloadRevision === reloadRevisionRef.current) {
      reloadRevisionRef.current = reloadRevision;
      return;
    }
    reloadRevisionRef.current = reloadRevision;
    void reloadInvoices();
  }, [reloadRevision, reloadInvoices]);

  const loadInvoiceDetail = useCallback(
    async (invoiceId: string, { retainExisting = false }: { retainExisting?: boolean } = {}) => {
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

      setStatus(undefined);
      setError(undefined);
      setRedeemMessage(undefined);
      setRedeemSteps([]);
      setRedeemModalOpen(false);

      if (!retainExisting) {
        setDetail(null);
      }
      setIsDetailLoading(true);

      try {
        const owner = normalizeHex(wallet.account);
        const storedBurns = loadStoredBurnArtifacts(owner);
        const invoiceIsSingle = isSingleInvoiceHex(invoiceId);
        const subIds = invoiceIsSingle ? [0] : Array.from({ length: 10 }, (_, idx) => idx);
        const seedHex = seed.seedHex;
        if (!seedHex) {
          const hasMissingBurn = subIds.some(
            (subId) => !getStoredBurnArtifact(storedBurns, invoiceId, subId),
          );
          if (hasMissingBurn) {
            setError(seed.error ?? 'Authorize privacy features by reconnecting your wallet.');
            setStatus(undefined);
            return;
          }
        }
        setStatus('Fetching invoice status…');

        const hubProvider = createProviderForHub(tokens.hub);
        const hubContract = getHubContract(tokens.hub.hubAddress, hubProvider);
        const indexer = createIndexerClient(config);

        const burnDetails: BurnDetail[] = [];

        const tokenProvider = createProviderForToken(connectedToken);
        const verifierContract = getVerifierContract(connectedToken.verifierAddress, tokenProvider);
        let storeMutated = false;

        for (const subId of subIds) {
          let burn = getStoredBurnArtifact(storedBurns, invoiceId, subId);
          if (!burn) {
            if (!seedHex) {
              throw new Error('Seed authorization required to derive burn artifacts.');
            }
            const secretAndTweak = invoiceIsSingle
              ? await deriveInvoiceSingle(seedHex, invoiceId, connectedToken.chainId, owner)
              : await deriveInvoiceBatch(seedHex, invoiceId, subId, connectedToken.chainId, owner);
            burn = await buildFullBurnAddress(
              connectedToken.chainId,
              owner,
              secretAndTweak.secret,
              secretAndTweak.tweak,
            );
            setStoredBurnArtifact(storedBurns, invoiceId, subId, burn);
            storeMutated = true;
          }

          const context = await collectRedeemContext({
            burn,
            tokens: availableTokens,
            indexer,
            verifierContract,
            hubContract,
            provider: hubProvider,
            indexerFetchLimit: config.indexerFetchLimit,
            eventBlockSpan: BigInt(config.eventBlockSpan),
          });

          burnDetails.push({
            subId,
            burn,
            context,
          });
        }

        if (storeMutated) {
          persistStoredBurnArtifacts(owner, storedBurns);
        }

        setDetail({
          invoiceId,
          isBatch: !invoiceIsSingle,
          burns: burnDetails,
          owner,
          chainId: connectedToken.chainId,
        });
        const firstBurnAddress = burnDetails[0]?.burn.burnAddress;
        if (firstBurnAddress) {
          setInvoiceLabels((prev) => {
            if (prev[invoiceId] === firstBurnAddress) {
              return prev;
            }
            return { ...prev, [invoiceId]: firstBurnAddress };
          });
        }
        setStatus('Invoice status loaded.');
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setStatus(undefined);
      } finally {
        setIsDetailLoading(false);
      }
    },
    [tokens, availableTokens, wallet, seed, config, connectedToken],
  );

  const handleInvoiceClick = useCallback(
    (invoiceId: string) => {
      if (selectedInvoice === invoiceId) {
        setSelectedInvoice(undefined);
        setRedeemMessage(undefined);
        setRedeemSteps([]);
        setRedeemModalOpen(false);
        return;
      }

      setSelectedInvoice(invoiceId);

      if (detail && detail.invoiceId === invoiceId) {
        return;
      }

      void loadInvoiceDetail(invoiceId);
    },
    [selectedInvoice, detail, loadInvoiceDetail],
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

        const hubProvider = createProviderForHub(tokens.hub);
        const hubContract = getHubContract(tokens.hub.hubAddress, hubProvider);
        const indexer = createIndexerClient(config);
        const tokenEntry = burnDetail.context.token;
        const tokenProvider = createProviderForToken(tokenEntry);
        const verifierRead = getVerifierContract(tokenEntry.verifierAddress, tokenProvider);

        const refreshedContext = await collectRedeemContext({
          burn: burnDetail.burn,
          tokens: availableTokens,
          indexer,
          verifierContract: verifierRead,
          hubContract,
          provider: hubProvider,
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
          setRedeemMessage(
            createTeleportSubmittedMessage({
              label: 'Single teleport submitted',
              txHash: tx.hash,
              chainId: burnDetail.burn.generalRecipient.chainId,
            }),
          );
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
          setRedeemMessage(
            createTeleportSubmittedMessage({
              label: 'Batch teleport submitted',
              txHash: tx.hash,
              chainId: burnDetail.burn.generalRecipient.chainId,
            }),
          );
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

  const renderInvoiceItem = (
    invoiceId: string,
    options: {
      key?: Key;
      ref?: (node: HTMLLIElement | null) => void;
      style?: CSSProperties;
      virtualized?: boolean;
      dataIndex?: number;
    } = {},
  ): JSX.Element => {
    const isSelected = invoiceId === selectedInvoice;
    const invoiceDetail = detail && detail.invoiceId === invoiceId ? detail : null;
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
    const cachedLabel = invoiceLabels[invoiceId];
    const summaryLabel =
      cachedLabel ??
      (invoiceDetail && invoiceDetail.burns.length > 0
        ? invoiceDetail.burns[0].burn.burnAddress
        : invoiceId);

    let itemStyle = options.style ? { ...options.style } : undefined;
    if (options.virtualized) {
      const spacing = '0.75rem';
      itemStyle = itemStyle
        ? { ...itemStyle, paddingBottom: spacing, boxSizing: 'border-box' }
        : { paddingBottom: spacing, boxSizing: 'border-box' };
    }

    return (
      <li
        key={options.key ?? invoiceId}
        ref={options.ref}
        className={isSelected ? 'accordion-item open' : 'accordion-item'}
        style={itemStyle}
        data-index={options.dataIndex ?? undefined}
      >
        <button
          type="button"
          className="accordion-trigger"
          onClick={() => handleInvoiceClick(invoiceId)}
          disabled={isLoading || isDetailLoading || seed.isDeriving || !seed.seedHex}
        >
          <div className="accordion-summary">
            <code className="mono">{summaryLabel}</code>
            <span className="badge">{isSingleInvoiceHex(invoiceId) ? 'Single' : 'Batch'}</span>
          </div>
          <span className="accordion-icon" aria-hidden="true">
            {isSelected ? '−' : '+'}
          </span>
        </button>
        {isSelected && (
          <div className="accordion-body">
            {invoiceDetail ? (
              <RedeemDetailSection
                title="Detail"
                metadata={[
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
                onReload={() => loadInvoiceDetail(invoiceId, { retainExisting: true })}
                reloadDisabled={isLoading || isDetailLoading}
                isReloading={isDetailLoading}
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
                    redeemDisabled:
                      isLoading || isDetailLoading || alreadyRedeemed || !hasEligibleValue,
                    redeemedLabel: noRedeemableTransfers
                      ? 'No Redeemable Transfers'
                      : alreadyRedeemed
                        ? 'Already redeemed'
                        : undefined,
                    eligibleValue: formatEligibleValue(burnDetail.context.totalEligibleValue),
                    pendingValue: formatEligibleValue(burnDetail.context.totalPendingValue),
                    chainSummaries: createChainSummaries(burnDetail.context.chains),
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
  };

  return (
    <section className="card">
      <header className="card-header">
        <h2>Previous Burn Addresses</h2>
      </header>
      {availableTokens.length === 0 ? (
        <div className="card-body">
          <p className="error">No token entries were loaded from tokens.json.</p>
        </div>
      ) : (
        <form className="card-body" onSubmit={handleLoadInvoices}>
          {!isWalletConnected && (
            <p className="hint">Connect your wallet in MetaMask to enable reloading.</p>
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
                {isLoading ? 'Reloading…' : 'Reload'}
              </button>
            </div>
          </footer>
        </form>
      )}

      {invoices.length > 0 && (
        <div className="card-section">
          {shouldVirtualize ? (
            <div ref={listParentRef} className="virtual-scroll-container">
              <ul
                className="list accordion virtualized-list"
                style={{
                  height: `${invoiceVirtualizer.getTotalSize()}px`,
                  position: 'relative',
                }}
              >
                {invoiceVirtualizer.getVirtualItems().map((virtualItem) => {
                  const invoiceId = invoices[virtualItem.index];
                  return renderInvoiceItem(invoiceId, {
                    key: virtualItem.key,
                    ref: (node) => invoiceVirtualizer.measureElement(node),
                    style: {
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      right: 0,
                      transform: `translateY(${virtualItem.start}px)`,
                    },
                    virtualized: true,
                    dataIndex: virtualItem.index,
                  });
                })}
              </ul>
            </div>
          ) : (
            <ul className="list accordion">{invoices.map((invoiceId) => renderInvoiceItem(invoiceId))}</ul>
          )}
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
