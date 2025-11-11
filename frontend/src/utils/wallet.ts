import chains from '@config/chains.json';
import type { SwitchChainOptions } from '@/app/providers/WalletProvider';
import type { TokenEntry } from '@zerc20/sdk';

type ChainListEntry = {
  chainId?: number;
  name?: string;
  nativeCurrency?: {
    name?: string;
    symbol?: string;
    decimals?: number;
  };
  explorers?: Array<{ url?: string }>;
};

interface NormalizedChainMetadata {
  name?: string;
  nativeCurrency?: {
    name: string;
    symbol: string;
    decimals: number;
  };
  explorerUrls: string[];
}

type SupportedChainId = bigint | number | string;

const metadataByChainId = new Map<string, NormalizedChainMetadata>();

const rawChains: unknown = chains;

if (Array.isArray(rawChains)) {
  for (const entry of rawChains as ChainListEntry[]) {
    const key = normalizeChainId(entry?.chainId);
    if (!key) {
      continue;
    }

    const explorerUrls = Array.isArray(entry?.explorers)
      ? entry!.explorers
          ?.map((explorer: { url?: string }) => (typeof explorer?.url === 'string' ? explorer.url.trim() : ''))
          .filter((url): url is string => Boolean(url && url.length > 0))
      : [];

    const nativeCurrency = normalizeNativeCurrency(entry?.nativeCurrency);

    metadataByChainId.set(key, {
      name: typeof entry?.name === 'string' ? entry.name : undefined,
      explorerUrls,
      nativeCurrency,
    });
  }
}

export function buildSwitchChainOptions(token: TokenEntry): SwitchChainOptions {
  const key = normalizeChainId(token.chainId);
  const metadata = key ? metadataByChainId.get(key) : undefined;
  const rpcUrls = token.rpcUrls
    .map((url: string) => url.trim())
    .filter((url): url is string => url.length > 0);

  return {
    chainName: metadata?.name ?? token.label,
    rpcUrls,
    nativeCurrency: metadata?.nativeCurrency ?? {
      name: 'Ether',
      symbol: 'ETH',
      decimals: 18,
    },
    blockExplorerUrls: metadata?.explorerUrls,
  };
}

function normalizeChainId(chainId: SupportedChainId | undefined): string | undefined {
  if (chainId === undefined || chainId === null) {
    return undefined;
  }
  if (typeof chainId === 'bigint') {
    return chainId.toString();
  }
  if (typeof chainId === 'number') {
    if (!Number.isFinite(chainId)) {
      return undefined;
    }
    return Math.trunc(chainId).toString();
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

function normalizeNativeCurrency(
  currency: ChainListEntry['nativeCurrency'],
): NormalizedChainMetadata['nativeCurrency'] | undefined {
  if (!currency) {
    return undefined;
  }
  const { name, symbol, decimals } = currency;
  if (typeof name !== 'string' || name.trim().length === 0) {
    return undefined;
  }
  if (typeof symbol !== 'string' || symbol.trim().length === 0) {
    return undefined;
  }
  if (typeof decimals !== 'number' || !Number.isFinite(decimals)) {
    return undefined;
  }
  return {
    name: name.trim(),
    symbol: symbol.trim(),
    decimals,
  };
}
