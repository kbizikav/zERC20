import type { HubEntry, TokenEntry, TokensFile } from '@services/sdk/registry/tokens.js';

export interface NormalizedTokens {
  hub?: HubEntry;
  tokens: TokenEntry[];
  raw: TokensFile;
}

export interface SingleTeleportArtifacts {
  localPk?: Uint8Array;
  localVk?: Uint8Array;
  globalPk?: Uint8Array;
  globalVk?: Uint8Array;
}

export interface BatchTeleportArtifacts {
  localPp?: Uint8Array;
  localVp?: Uint8Array;
  globalPp?: Uint8Array;
  globalVp?: Uint8Array;
}

export interface TeleportArtifacts {
  single: SingleTeleportArtifacts;
  batch: BatchTeleportArtifacts;
}
