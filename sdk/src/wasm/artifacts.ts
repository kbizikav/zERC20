import { ensureFetch } from '../utils/http.js';

export interface SingleTeleportWasmArtifacts {
  localPk: Uint8Array;
  localVk: Uint8Array;
  globalPk: Uint8Array;
  globalVk: Uint8Array;
}

export interface BatchTeleportWasmArtifacts {
  localPp: Uint8Array;
  localVp: Uint8Array;
  globalPp: Uint8Array;
  globalVp: Uint8Array;
}

export interface TeleportWasmArtifacts {
  single: SingleTeleportWasmArtifacts;
  batch: BatchTeleportWasmArtifacts;
}

export interface LoadTeleportArtifactsOptions {
  fetchImpl?: typeof fetch;
}

type ArtifactFileMap = Record<string, string>;

const SINGLE_ARTIFACT_FILES = {
  localPk: 'withdraw_local_groth16_pk.bin',
  localVk: 'withdraw_local_groth16_vk.bin',
  globalPk: 'withdraw_global_groth16_pk.bin',
  globalVk: 'withdraw_global_groth16_vk.bin',
} as const satisfies ArtifactFileMap;

const BATCH_ARTIFACT_FILES = {
  localPp: 'withdraw_local_nova_pp.bin',
  localVp: 'withdraw_local_nova_vp.bin',
  globalPp: 'withdraw_global_nova_pp.bin',
  globalVp: 'withdraw_global_nova_vp.bin',
} as const satisfies ArtifactFileMap;

function artifactUrl(filename: string): string {
  return new URL(`../assets/artifacts/${filename}`, import.meta.url).toString();
}

function buildArtifactUrls<T extends ArtifactFileMap>(files: T): T {
  const entries = Object.entries(files).map(([key, file]) => [key, artifactUrl(file)]);
  return Object.fromEntries(entries) as T;
}

const SINGLE_ARTIFACT_URLS = buildArtifactUrls(SINGLE_ARTIFACT_FILES);
const BATCH_ARTIFACT_URLS = buildArtifactUrls(BATCH_ARTIFACT_FILES);
const SINGLE_ARTIFACT_URL_SET = new Set<string>(Object.values(SINGLE_ARTIFACT_URLS));
const BATCH_ARTIFACT_URL_SET = new Set<string>(Object.values(BATCH_ARTIFACT_URLS));

const binaryCache = new Map<string, Promise<Uint8Array>>();
let singleArtifactsPromise: Promise<SingleTeleportWasmArtifacts> | undefined;
let batchArtifactsPromise: Promise<BatchTeleportWasmArtifacts> | undefined;

export function clearTeleportArtifactCache(url?: string): void {
  if (url) {
    binaryCache.delete(url);
    if (SINGLE_ARTIFACT_URL_SET.has(url)) {
      singleArtifactsPromise = undefined;
    }
    if (BATCH_ARTIFACT_URL_SET.has(url)) {
      batchArtifactsPromise = undefined;
    }
    return;
  }
  singleArtifactsPromise = undefined;
  batchArtifactsPromise = undefined;
  binaryCache.clear();
}

async function fetchBinary(url: string, fetchFn: typeof fetch): Promise<Uint8Array> {
  if (!binaryCache.has(url)) {
    const promise = (async () => {
      const response = await fetchFn(url);
      if (!response.ok) {
        throw new Error(`Failed to load artifact from ${url} (${response.status})`);
      }
      const buffer = await response.arrayBuffer();
      return new Uint8Array(buffer);
    })();
    binaryCache.set(url, promise);
  }
  return binaryCache.get(url) as Promise<Uint8Array>;
}

async function loadArtifactGroup<T extends ArtifactFileMap>(
  paths: T,
  fetchFn: typeof fetch,
): Promise<{ [K in keyof T]: Uint8Array }> {
  const entries = await Promise.all(
    Object.entries(paths).map(async ([key, url]) => {
      const bytes = await fetchBinary(url, fetchFn);
      return [key, bytes] as const;
    }),
  );
  return Object.fromEntries(entries) as { [K in keyof T]: Uint8Array };
}

export async function loadSingleTeleportArtifacts(
  options: LoadTeleportArtifactsOptions = {},
): Promise<SingleTeleportWasmArtifacts> {
  if (!singleArtifactsPromise) {
    const fetchFn = options.fetchImpl ?? ensureFetch();
    singleArtifactsPromise = loadArtifactGroup(
      SINGLE_ARTIFACT_URLS,
      fetchFn,
    ) as Promise<SingleTeleportWasmArtifacts>;
  }
  return singleArtifactsPromise;
}

export async function loadBatchTeleportArtifacts(
  options: LoadTeleportArtifactsOptions = {},
): Promise<BatchTeleportWasmArtifacts> {
  if (!batchArtifactsPromise) {
    const fetchFn = options.fetchImpl ?? ensureFetch();
    batchArtifactsPromise = loadArtifactGroup(
      BATCH_ARTIFACT_URLS,
      fetchFn,
    ) as Promise<BatchTeleportWasmArtifacts>;
  }
  return batchArtifactsPromise;
}

export async function loadTeleportArtifacts(
  options: LoadTeleportArtifactsOptions = {},
): Promise<TeleportWasmArtifacts> {
  const [single, batch] = await Promise.all([
    loadSingleTeleportArtifacts(options),
    loadBatchTeleportArtifacts(options),
  ]);
  return { single, batch };
}
