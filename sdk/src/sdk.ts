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

export interface Zerc20SdkOptions {
  wasm?: WasmRuntime | WasmRuntimeOptions;
  proofs?: ProofServiceOptions;
  decider?: {
    baseUrl: string;
  } & DeciderClientOptions;
}

export class Zerc20Sdk {
  readonly wasm: WasmRuntime;
  readonly proofs: ProofService;
  readonly decider?: HttpDeciderClient;

  constructor(options: Zerc20SdkOptions = {}) {
    this.wasm = options.wasm instanceof WasmRuntime ? options.wasm : new WasmRuntime(options.wasm);
    this.proofs = new ProofService(this.wasm, options.proofs);
    if (options.decider) {
      this.decider = new HttpDeciderClient(options.decider.baseUrl, options.decider);
    }
  }

  createStealthClient(options: StealthClientConfig): StealthCanisterClient {
    return new StealthCanisterClient(
      options.agent,
      asPrincipal(options.storageCanisterId),
      asPrincipal(options.keyManagerCanisterId),
    );
  }
}

export function createSdk(options: Zerc20SdkOptions = {}): Zerc20Sdk {
  return new Zerc20Sdk(options);
}
