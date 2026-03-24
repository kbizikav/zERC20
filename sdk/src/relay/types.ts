import type { Hex } from 'viem';

export interface RelayTeleportParams {
  isSingle: boolean;
  isGlobal: boolean;
  rootHint: bigint;
  chainId: bigint;
  recipient: Hex;
  tweak: Hex;
  proof: Hex;
  relayerFee: bigint;
  maxFee: bigint;
  deadline: bigint;
  signature: Hex;
}

export interface RelayFeeEstimate {
  relayerFee: bigint;
}

export interface RelayResult {
  txHash: string;
}
