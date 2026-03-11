import type { Address, Hex, PublicClient } from 'viem';

export interface EstimateRelayerFeeParams {
  chainId: number;
  feeToken: Address;
  gasLimit: bigint;
  liquidityManagerAddress: Address;
  publicClient: PublicClient;
}

export interface EstimateRelayerFeeResult {
  relayerFee: bigint;
  gelatoFee: bigint;
  maxGelatoFee: bigint;
  unwrapFee: bigint;
}

export interface SubmitTeleportRelayParams {
  chainId: number;
  gelatoRelayAddress: Address;
  feeToken: Address;
  calldata: Hex;
  apiKey?: string;
  gasLimit?: bigint;
}

export interface SubmitTeleportRelayResult {
  taskId: string;
}

export enum RelayTaskState {
  CheckPending = 'CheckPending',
  ExecPending = 'ExecPending',
  WaitingForConfirmation = 'WaitingForConfirmation',
  ExecSuccess = 'ExecSuccess',
  ExecReverted = 'ExecReverted',
  Cancelled = 'Cancelled',
}

export interface RelayTaskResult {
  taskId: string;
  taskState: RelayTaskState;
  transactionHash?: string;
  blockNumber?: number;
  executionDate?: string;
  lastCheckMessage?: string;
}

export interface WaitForRelayTaskOptions {
  polls?: number;
  intervalMs?: number;
}
