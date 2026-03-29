import type { Hex } from "viem";

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

export interface SwapQuoteParams {
  chainId: number;
  /** Token amount in smallest unit. */
  amount: bigint;
}

export interface SwapQuote {
  /** Native token amount the user will receive (wei). */
  nativeAmount: bigint;
  /** Fee in basis points. */
  feeBps: number;
  /** Relayer gas fee deducted from the native output (wei). */
  relayerFee: bigint;
}

export interface RelaySwapParams {
  chainId: number;
  /** Token amount to swap (smallest unit). */
  tokenAmount: bigint;
  /** Minimum native output the user will accept (slippage protection). */
  minNativeAmount: bigint;
  /** Address to receive native tokens. */
  recipient: Hex;
  /** Address that signed the permit (token owner). */
  owner: Hex;
  permitDeadline: bigint;
  permitV: number;
  permitR: Hex;
  permitS: Hex;
}

export interface SwapResult {
  txHash: string;
}
