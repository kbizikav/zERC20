import chains from '@config/chains.json';

type ExplorerEntry = {
  url?: string;
  standard?: string;
};

type ChainEntry = {
  chainId?: number | string;
  explorers?: ExplorerEntry[];
};

type SupportedChainId = bigint | number | string;

interface ExplorerMapEntry {
  baseUrl: string;
  standard?: string;
}

const explorerByChainId: Map<string, ExplorerMapEntry> = new Map();

const rawChains: unknown = chains;

if (Array.isArray(rawChains)) {
  for (const chain of rawChains as ChainEntry[]) {
    const chainId = normalizeChainId(chain.chainId);
    if (!chainId) {
      continue;
    }

    const explorers = Array.isArray(chain.explorers) ? chain.explorers : [];
    const normalizedExplorers = explorers
      .map((entry): ExplorerMapEntry | undefined => {
        if (!entry || typeof entry.url !== 'string') {
          return undefined;
        }
        const trimmed = entry.url.trim();
        if (trimmed.length === 0) {
          return undefined;
        }
        const sanitized = trimmed.replace(/\/+$/, '');
        const standard = typeof entry.standard === 'string' ? entry.standard.trim() : undefined;
        return { baseUrl: sanitized, standard };
      })
      .filter((value): value is ExplorerMapEntry => Boolean(value));

    if (normalizedExplorers.length === 0) {
      continue;
    }

    const preferred =
      normalizedExplorers.find((entry) => entry.standard?.toUpperCase() === 'EIP3091') ??
      normalizedExplorers[0];

    explorerByChainId.set(chainId, preferred);
  }
}

export function getExplorerTxUrl(chainId: SupportedChainId, txHash: string): string | undefined {
  const key = normalizeChainId(chainId);
  if (!key) {
    return undefined;
  }
  const explorer = explorerByChainId.get(key);
  if (!explorer) {
    return undefined;
  }

  const normalizedHash = normalizeTransactionHash(txHash);
  if (!normalizedHash) {
    return undefined;
  }

  if (explorer.standard?.toUpperCase() !== 'EIP3091') {
    return `${explorer.baseUrl}/${normalizedHash}`;
  }

  return `${explorer.baseUrl}/tx/${normalizedHash}`;
}

function normalizeChainId(chainId: SupportedChainId | undefined): string | undefined {
  if (typeof chainId === 'bigint') {
    return chainId.toString();
  }
  if (typeof chainId === 'number') {
    if (!Number.isFinite(chainId)) {
      return undefined;
    }
    if (!Number.isSafeInteger(chainId)) {
      return Math.trunc(chainId).toString();
    }
    return chainId.toString();
  }
  if (typeof chainId === 'string') {
    const trimmed = chainId.trim();
    if (trimmed.length === 0) {
      return undefined;
    }
    if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
      try {
        return BigInt(trimmed).toString();
      } catch {
        return undefined;
      }
    }
    if (/^\d+$/.test(trimmed)) {
      return trimmed.replace(/^0+/, '') || '0';
    }
    return trimmed;
  }
  return undefined;
}

function normalizeTransactionHash(txHash: string): string | undefined {
  if (typeof txHash !== 'string') {
    return undefined;
  }
  const trimmed = txHash.trim();
  if (trimmed.length === 0) {
    return undefined;
  }
  if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
    return trimmed;
  }
  const prefixed = trimmed.startsWith('0x') ? trimmed : `0x${trimmed}`;
  if (/^0x[0-9a-fA-F]+$/.test(prefixed)) {
    return prefixed;
  }
  return undefined;
}

