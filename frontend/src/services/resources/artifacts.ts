import type { ArtifactPaths } from '@config/appConfig';
import type { BatchTeleportArtifacts, SingleTeleportArtifacts, TeleportArtifacts } from '@/types/app';

const binaryCache = new Map<string, Promise<Uint8Array>>();

async function fetchBinary(url: string): Promise<Uint8Array> {
  if (!binaryCache.has(url)) {
    const promise = fetch(url).then(async (response) => {
      if (!response.ok) {
        throw new Error(`Failed to load artifact from ${url} (${response.status})`);
      }
      const buffer = await response.arrayBuffer();
      return new Uint8Array(buffer);
    });
    binaryCache.set(url, promise);
  }
  return binaryCache.get(url) as Promise<Uint8Array>;
}

async function loadArtifactGroup<T extends { [K in keyof T]: string }>(
  paths: T,
): Promise<Record<keyof T, Uint8Array>> {
  const entries = await Promise.all(
    (Object.entries(paths) as [keyof T & string, string][]).map(async ([key, url]) => {
      const bytes = await fetchBinary(url);
      return [key, bytes] as const;
    }),
  );
  return Object.fromEntries(entries) as Record<keyof T, Uint8Array>;
}

export async function loadTeleportArtifacts(paths: ArtifactPaths): Promise<TeleportArtifacts> {
  const [single, batch] = await Promise.all([
    loadArtifactGroup(paths.single),
    loadArtifactGroup(paths.batch),
  ]);

  return {
    single: single as SingleTeleportArtifacts,
    batch: batch as BatchTeleportArtifacts,
  };
}

export function clearArtifactsCache(): void {
  binaryCache.clear();
}
