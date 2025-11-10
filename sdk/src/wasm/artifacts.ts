import { ensureFetch } from '../utils/http.js';

export interface TeleportArtifactUrlMap {
  [key: string]: string;
}

export interface TeleportArtifactPaths {
  single: TeleportArtifactUrlMap;
  batch: TeleportArtifactUrlMap;
}

export interface SingleTeleportWasmArtifacts {
  localPk?: Uint8Array;
  localVk?: Uint8Array;
  globalPk?: Uint8Array;
  globalVk?: Uint8Array;
}

export interface BatchTeleportWasmArtifacts {
  localPp?: Uint8Array;
  localVp?: Uint8Array;
  globalPp?: Uint8Array;
  globalVp?: Uint8Array;
}

export interface TeleportWasmArtifacts {
  single: SingleTeleportWasmArtifacts;
  batch: BatchTeleportWasmArtifacts;
}

export interface LoadTeleportArtifactsOptions {
  fetchImpl?: typeof fetch;
}

const binaryCache = new Map<string, Promise<Uint8Array>>();

export function clearTeleportArtifactCache(url?: string): void {
  if (url) {
    binaryCache.delete(url);
    return;
  }
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

async function loadArtifactGroup(paths: TeleportArtifactUrlMap, fetchFn: typeof fetch): Promise<Record<string, Uint8Array>> {
  const entries = await Promise.all(
    Object.entries(paths).map(async ([key, url]) => {
      const bytes = await fetchBinary(url, fetchFn);
      return [key, bytes] as const;
    }),
  );
  return Object.fromEntries(entries);
}

export async function loadTeleportArtifacts(
  paths: TeleportArtifactPaths,
  options: LoadTeleportArtifactsOptions = {},
): Promise<TeleportWasmArtifacts> {
  const fetchFn = options.fetchImpl ?? ensureFetch();
  const [single, batch] = await Promise.all([
    loadArtifactGroup(paths.single, fetchFn),
    loadArtifactGroup(paths.batch, fetchFn),
  ]);
  return {
    single: single as SingleTeleportWasmArtifacts,
    batch: batch as BatchTeleportWasmArtifacts,
  };
}
