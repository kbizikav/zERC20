import type { Agent } from '@dfinity/agent';
import { Principal } from '@dfinity/principal';

import type { DeciderClientOptions } from './decider/prover.js';
import { HttpDeciderClient } from './decider/prover.js';
import { StealthCanisterClient } from './ic/client.js';
import type { WasmRuntimeOptions } from './wasm/index.js';
import { WasmRuntime } from './wasm/index.js';
import type { ProofServiceOptions } from './zkp/index.js';
import { ProofService } from './zkp/index.js';

export interface StealthClientConfig {
  agent: Agent;
  storageCanisterId: string | Principal;
  keyManagerCanisterId: string | Principal;
}

function asPrincipal(value: string | Principal): Principal {
  return typeof value === 'string' ? Principal.fromText(value) : value;
}

export interface StealthClientFactoryOptions {
  defaults?: Partial<StealthClientConfig>;
}

interface NormalizedStealthClientConfig {
  agent: Agent;
  storageCanisterId: Principal;
  keyManagerCanisterId: Principal;
}

function ensureValue<T>(value: T | undefined, field: string): T {
  if (value === undefined || value === null) {
    throw new Error(`stealth client ${field} is required`);
  }
  return value;
}

function requireStealthConfig(config: Partial<StealthClientConfig>): StealthClientConfig {
  return {
    agent: ensureValue(config.agent, 'agent'),
    storageCanisterId: ensureValue(config.storageCanisterId, 'storageCanisterId'),
    keyManagerCanisterId: ensureValue(config.keyManagerCanisterId, 'keyManagerCanisterId'),
  };
}

function normalizeStealthConfig(config: StealthClientConfig): NormalizedStealthClientConfig {
  return {
    agent: config.agent,
    storageCanisterId: asPrincipal(config.storageCanisterId),
    keyManagerCanisterId: asPrincipal(config.keyManagerCanisterId),
  };
}

export class StealthClientFactory {
  constructor(private readonly defaults: Partial<StealthClientConfig> = {}) {}

  create(config?: Partial<StealthClientConfig>): StealthCanisterClient {
    const merged: Partial<StealthClientConfig> = {
      ...this.defaults,
      ...config,
    };
    const normalized = normalizeStealthConfig(requireStealthConfig(merged));
    return new StealthCanisterClient(
      normalized.agent,
      normalized.storageCanisterId,
      normalized.keyManagerCanisterId,
    );
  }

  withDefaults(defaults: Partial<StealthClientConfig>): StealthClientFactory {
    return new StealthClientFactory({
      ...this.defaults,
      ...defaults,
    });
  }
}

type ProofServiceInit = ProofService | ProofServiceOptions | undefined;
type DeciderInit = HttpDeciderClient | (DeciderClientOptions & { baseUrl: string }) | undefined;
type StealthFactoryInit = StealthClientFactory | StealthClientFactoryOptions | undefined;

export interface Zerc20SdkOptions {
  wasm?: WasmRuntime | WasmRuntimeOptions;
  proofs?: ProofServiceInit;
  decider?: DeciderInit;
  stealth?: StealthFactoryInit;
}

export class Zerc20Sdk {
  readonly wasm: WasmRuntime;
  readonly proofs: ProofService;
  readonly decider?: HttpDeciderClient;
  readonly stealth: StealthClientFactory;

  constructor(options: Zerc20SdkOptions = {}) {
    this.wasm = resolveWasmRuntime(options.wasm);
    this.proofs = resolveProofService(this.wasm, options.proofs);
    this.decider = resolveDeciderClient(options.decider);
    this.stealth = resolveStealthFactory(options.stealth);
  }

  createStealthClient(options?: Partial<StealthClientConfig>): StealthCanisterClient {
    return this.stealth.create(options);
  }
}

export function createSdk(options: Zerc20SdkOptions = {}): Zerc20Sdk {
  return new Zerc20Sdk(options);
}

function resolveWasmRuntime(input?: WasmRuntime | WasmRuntimeOptions): WasmRuntime {
  return input instanceof WasmRuntime ? input : new WasmRuntime(input);
}

function resolveProofService(wasm: WasmRuntime, input: ProofServiceInit): ProofService {
  if (input instanceof ProofService) {
    return input;
  }
  return new ProofService(wasm, input);
}

function resolveDeciderClient(input: DeciderInit): HttpDeciderClient | undefined {
  if (!input) {
    return undefined;
  }
  if (input instanceof HttpDeciderClient) {
    return input;
  }
  return new HttpDeciderClient(input.baseUrl, input);
}

function resolveStealthFactory(input: StealthFactoryInit): StealthClientFactory {
  if (!input) {
    return new StealthClientFactory();
  }
  if (input instanceof StealthClientFactory) {
    return input;
  }
  return new StealthClientFactory(input.defaults);
}
