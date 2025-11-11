import { z } from 'zod';

export interface AppConfig {
  indexerUrl: string;
  deciderUrl: string;
  icReplicaUrl: string;
  storageCanisterId: string;
  keyManagerCanisterId: string;
  indexerFetchLimit: number;
  eventBlockSpan: number;
  scanPageSize: number;
  authorizationTtlSeconds: number;
  tokenSymbol: string;
}

export interface RuntimeConfig {
  app: AppConfig;
  tokensCompressed: string;
}

const optionalNumber = (fallback: number) =>
  z
    .string()
    .trim()
    .transform((value, ctx) => {
      if (value.length === 0) {
        return fallback;
      }
      const parsed = Number(value);
      if (!Number.isFinite(parsed)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: `Expected numeric value, received '${value}'`,
        });
        return z.NEVER;
      }
      return parsed;
    })
    .catch(() => fallback);

const envSchema = z.object({
  VITE_INDEXER_URL: z.string().trim().min(1, 'VITE_INDEXER_URL is required'),
  VITE_DECIDER_URL: z.string().trim().min(1, 'VITE_DECIDER_URL is required'),
  VITE_IC_REPLICA_URL: z.string().trim().min(1, 'VITE_IC_REPLICA_URL is required'),
  VITE_STORAGE_CANISTER_ID: z.string().trim().min(1, 'VITE_STORAGE_CANISTER_ID is required'),
  VITE_KEY_MANAGER_CANISTER_ID: z
    .string()
    .trim()
    .min(1, 'VITE_KEY_MANAGER_CANISTER_ID is required'),
  VITE_INDEXER_FETCH_LIMIT: optionalNumber(20),
  VITE_EVENT_BLOCK_SPAN: optionalNumber(5_000),
  VITE_SCAN_PAGE_SIZE: optionalNumber(100),
  VITE_AUTHORIZATION_TTL_SECONDS: optionalNumber(600),
  VITE_TOKENS_COMPRESSED: z
    .string()
    .trim()
    .min(1, 'VITE_TOKENS_COMPRESSED is required; run scripts/encode-tokens.sh')
    .regex(/^[A-Za-z0-9+/=]+$/, 'VITE_TOKENS_COMPRESSED must be base64-encoded'),
  VITE_TOKEN_SYMBOL: z
    .string()
    .trim()
    .min(1, 'VITE_TOKEN_SYMBOL must not be empty')
    .catch('zUSD'),
});

export function resolveRuntimeConfig(env: ImportMetaEnv): RuntimeConfig {
  const coercedEnv = Object.fromEntries(
    Object.entries(env ?? {}).map(([key, value]) => [key, typeof value === 'string' ? value : '']),
  );

  const parsed = envSchema.parse(coercedEnv);

  const app: AppConfig = {
    indexerUrl: parsed.VITE_INDEXER_URL,
    deciderUrl: parsed.VITE_DECIDER_URL,
    icReplicaUrl: parsed.VITE_IC_REPLICA_URL,
    storageCanisterId: parsed.VITE_STORAGE_CANISTER_ID,
    keyManagerCanisterId: parsed.VITE_KEY_MANAGER_CANISTER_ID,
    indexerFetchLimit: parsed.VITE_INDEXER_FETCH_LIMIT,
    eventBlockSpan: parsed.VITE_EVENT_BLOCK_SPAN,
    scanPageSize: parsed.VITE_SCAN_PAGE_SIZE,
    authorizationTtlSeconds: parsed.VITE_AUTHORIZATION_TTL_SECONDS,
    tokenSymbol: parsed.VITE_TOKEN_SYMBOL,
  };

  return { app, tokensCompressed: parsed.VITE_TOKENS_COMPRESSED };
}
