import type { Agent, HttpAgentOptions } from '@dfinity/agent';
import { HttpAgent } from '@dfinity/agent';
import { Principal } from '@dfinity/principal';

import type { DeciderClientOptions } from '../decider/prover.js';
import { HttpDeciderClient } from '../decider/prover.js';
import { StealthCanisterClient } from './client.js';

export interface StealthClientConnectionOptions {
  icReplicaUrl: string;
  storageCanisterId: string;
  keyManagerCanisterId: string;
  agent?: Agent;
  agentOptions?: HttpAgentOptions;
  fetchRootKey?: boolean;
}

export interface CachedStealthClientOptions extends StealthClientConnectionOptions {
  cacheKey?: string;
  cache?: Map<string, Promise<StealthCanisterClient>>;
}

const defaultStealthCache = new Map<string, Promise<StealthCanisterClient>>();

export function isLocalReplica(url: string): boolean {
  return /localhost|127\.0\.0\.1/.test(url);
}

async function buildAgent(options: StealthClientConnectionOptions): Promise<Agent> {
  if (options.agent) {
    return options.agent;
  }
  const agent = new HttpAgent({
    ...options.agentOptions,
    host: options.icReplicaUrl,
  });
  const shouldFetchRootKey = options.fetchRootKey ?? isLocalReplica(options.icReplicaUrl);
  if (shouldFetchRootKey) {
    await agent.fetchRootKey();
  }
  return agent;
}

async function instantiateStealthClient(options: StealthClientConnectionOptions): Promise<StealthCanisterClient> {
  const agent = await buildAgent(options);
  const storageId = Principal.fromText(options.storageCanisterId);
  const keyManagerId = Principal.fromText(options.keyManagerCanisterId);
  return new StealthCanisterClient(agent, storageId, keyManagerId);
}

export async function createStealthClient(options: StealthClientConnectionOptions): Promise<StealthCanisterClient> {
  return instantiateStealthClient(options);
}

export async function getStealthClientFromConfig(options: CachedStealthClientOptions): Promise<StealthCanisterClient> {
  const cache = options.cache ?? defaultStealthCache;
  const cacheKey =
    options.cacheKey ?? `${options.icReplicaUrl}|${options.storageCanisterId}|${options.keyManagerCanisterId}`;
  if (!cache.has(cacheKey)) {
    cache.set(
      cacheKey,
      instantiateStealthClient({
        icReplicaUrl: options.icReplicaUrl,
        storageCanisterId: options.storageCanisterId,
        keyManagerCanisterId: options.keyManagerCanisterId,
        agent: options.agent,
        agentOptions: options.agentOptions,
        fetchRootKey: options.fetchRootKey,
      }),
    );
  }
  return cache.get(cacheKey) as Promise<StealthCanisterClient>;
}

export function clearStealthClientCache(
  cache?: Map<string, Promise<StealthCanisterClient>>,
  key?: string,
): void {
  const target = cache ?? defaultStealthCache;
  if (key) {
    target.delete(key);
    return;
  }
  target.clear();
}

export interface DeciderClientConfig extends DeciderClientOptions {
  baseUrl: string;
  cacheKey?: string;
  cache?: Map<string, HttpDeciderClient>;
}

const defaultDeciderCache = new Map<string, HttpDeciderClient>();

export function getDeciderClient(config: DeciderClientConfig): HttpDeciderClient {
  const cache = config.cache ?? defaultDeciderCache;
  const cacheKey = config.cacheKey ?? config.baseUrl;
  if (!cache.has(cacheKey)) {
    const { baseUrl, cacheKey: _cacheKey, cache: _cache, ...options } = config;
    cache.set(cacheKey, new HttpDeciderClient(baseUrl, options));
  }
  return cache.get(cacheKey) as HttpDeciderClient;
}

export function clearDeciderClientCache(cache?: Map<string, HttpDeciderClient>, key?: string): void {
  const target = cache ?? defaultDeciderCache;
  if (key) {
    target.delete(key);
    return;
  }
  target.clear();
}
