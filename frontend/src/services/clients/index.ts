import { HttpAgent } from '@dfinity/agent';
import { Principal } from '@dfinity/principal';
import { HttpDeciderClient, StealthCanisterClient } from '@zerc20/sdk';
import type { AppConfig } from '@config/appConfig';

function isLocalhost(url: string): boolean {
  return /localhost|127\.0\.0\.1/.test(url);
}

const stealthClientCache = new Map<string, Promise<StealthCanisterClient>>();

export async function getStealthClient(config: AppConfig): Promise<StealthCanisterClient> {
  const cacheKey = `${config.icReplicaUrl}|${config.storageCanisterId}|${config.keyManagerCanisterId}`;
  if (!stealthClientCache.has(cacheKey)) {
    const promise = (async () => {
      const agent = new HttpAgent({ host: config.icReplicaUrl });
      if (isLocalhost(config.icReplicaUrl)) {
        await agent.fetchRootKey();
      }
      const storageId = Principal.fromText(config.storageCanisterId);
      const keyManagerId = Principal.fromText(config.keyManagerCanisterId);
      return new StealthCanisterClient(agent, storageId, keyManagerId);
    })();
    stealthClientCache.set(cacheKey, promise);
  }
  return stealthClientCache.get(cacheKey) as Promise<StealthCanisterClient>;
}

export function createDeciderClient(config: AppConfig): HttpDeciderClient {
  return new HttpDeciderClient(config.deciderUrl);
}

export function clearStealthClientCache(): void {
  stealthClientCache.clear();
}
