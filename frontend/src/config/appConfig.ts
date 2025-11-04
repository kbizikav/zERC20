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

export interface ArtifactLocationMap {
  localPk: string;
  localVk: string;
  globalPk: string;
  globalVk: string;
}

export interface NovaArtifactLocationMap {
  localPp: string;
  localVp: string;
  globalPp: string;
  globalVp: string;
}

export interface ArtifactPaths {
  basePath: string;
  single: ArtifactLocationMap;
  batch: NovaArtifactLocationMap;
}

export interface ResourceConfig {
  tokensPath: string;
  artifacts: ArtifactPaths;
}

export interface RuntimeConfig {
  app: AppConfig;
  resources: ResourceConfig;
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
  VITE_TOKENS_PATH: z.string().trim().default('/config/tokens.json'),
  VITE_ARTIFACTS_BASE_PATH: z.string().trim().default('/artifacts'),
  VITE_TOKEN_SYMBOL: z
    .string()
    .trim()
    .min(1, 'VITE_TOKEN_SYMBOL must not be empty')
    .catch('zUSD'),
});

const singleArtifactFiles: ArtifactLocationMap = {
  localPk: 'withdraw_local_groth16_pk.bin',
  localVk: 'withdraw_local_groth16_vk.bin',
  globalPk: 'withdraw_global_groth16_pk.bin',
  globalVk: 'withdraw_global_groth16_vk.bin',
};

const batchArtifactFiles: NovaArtifactLocationMap = {
  localPp: 'withdraw_local_nova_pp.bin',
  localVp: 'withdraw_local_nova_vp.bin',
  globalPp: 'withdraw_global_nova_pp.bin',
  globalVp: 'withdraw_global_nova_vp.bin',
};

function joinUrl(base: string, path: string): string {
  if (path.startsWith('http://') || path.startsWith('https://')) {
    return path;
  }
  const normalizedBase = base.endsWith('/') ? base : `${base}/`;
  return `${normalizedBase}${path}`;
}

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

  const basePath = parsed.VITE_ARTIFACTS_BASE_PATH.startsWith('http')
    ? parsed.VITE_ARTIFACTS_BASE_PATH
    : parsed.VITE_ARTIFACTS_BASE_PATH.replace(/\/$/, '');

  const withBase = <T extends { [K in keyof T]: string }>(files: T): T => {
    const entries = Object.entries(files) as [keyof T & string, string][];
    const withJoined = entries.map(([key, file]) => [key, joinUrl(basePath, file)]);
    return Object.fromEntries(withJoined) as T;
  };

  const resources: ResourceConfig = {
    tokensPath: parsed.VITE_TOKENS_PATH,
    artifacts: {
      basePath,
      single: withBase(singleArtifactFiles),
      batch: withBase(batchArtifactFiles),
    },
  };

  return { app, resources };
}
