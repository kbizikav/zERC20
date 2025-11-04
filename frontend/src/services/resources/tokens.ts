import { normalizeTokensFile, TokensFile } from '@services/sdk/registry/tokens.js';
import type { NormalizedTokens } from '@/types/app';

const tokenCache = new Map<string, Promise<NormalizedTokens>>();

function asNormalizedTokens(file: TokensFile): NormalizedTokens {
  const normalized = normalizeTokensFile({ ...file, tokens: [...file.tokens] });
  return {
    raw: file,
    tokens: normalized.tokens,
    hub: normalized.hub,
  };
}

export function loadTokens(tokensPath: string): Promise<NormalizedTokens> {
  if (!tokenCache.has(tokensPath)) {
    const promise = fetch(tokensPath)
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`Failed to load tokens file (${response.status}) from ${tokensPath}`);
        }
        return (await response.json()) as TokensFile;
      })
      .then(asNormalizedTokens);
    tokenCache.set(tokensPath, promise);
  }
  return tokenCache.get(tokensPath) as Promise<NormalizedTokens>;
}

export function clearTokensCache(): void {
  tokenCache.clear();
}
