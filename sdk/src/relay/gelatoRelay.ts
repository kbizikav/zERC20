import { GelatoRelay, type CallWithSyncFeeRequest } from '@gelatonetwork/relay-sdk';
import { encodeFunctionData, type Address, type Hex, type PublicClient } from 'viem';

import { gelatoRelayAbi } from './abi.js';
import type {
  EstimateRelayerFeeParams,
  EstimateRelayerFeeResult,
  RelayTaskResult,
  SubmitTeleportRelayParams,
  SubmitTeleportRelayResult,
  WaitForRelayTaskOptions,
} from './types.js';
import { RelayTaskState } from './types.js';

const GELATO_RELAY_GAS_OVERHEAD = 150_000n;
const DEFAULT_GAS_LIMIT = 1_000_000n;
const RELAY_FEE_BUFFER_BPS = 500n; // 5% buffer
const BPS_DENOMINATOR = 10_000n;

const DEFAULT_POLLS = 40;
const DEFAULT_INTERVAL_MS = 3_000;

const liquidityManagerAbi = [
  {
    type: 'function',
    name: 'quoteUnwrapFee',
    inputs: [{ name: 'amount', type: 'uint256', internalType: 'uint256' }],
    outputs: [{ name: 'feeAmount', type: 'uint256', internalType: 'uint256' }],
    stateMutability: 'view',
  },
] as const;

/**
 * Estimates the total relayerFee (in zERC20) that covers:
 * - Gelato gas cost (in underlying, estimated via relay SDK oracle)
 * - LiquidityManager unwrap fee
 * - Safety buffer (5%)
 */
export async function estimateRelayerFee(params: EstimateRelayerFeeParams): Promise<EstimateRelayerFeeResult> {
  const { chainId, feeToken, gasLimit, liquidityManagerAddress, publicClient } = params;

  const relay = new GelatoRelay();
  const totalGasLimit = gasLimit + GELATO_RELAY_GAS_OVERHEAD;
  const gelatoFee = await relay.getEstimatedFee(BigInt(chainId), feeToken, totalGasLimit, false);

  // gelatoFee is in underlying units. We need to find how much zERC20 to unwrap
  // to get at least gelatoFee in underlying after the unwrap fee.
  // unwrapFee = quoteUnwrapFee(amount) where amount is zERC20 input.
  // Net underlying = amount - unwrapFee(amount)
  // We want: amount - unwrapFee(amount) >= gelatoFee
  // Start with a slightly higher estimate and iterate.
  let candidate = gelatoFee;
  for (let i = 0; i < 5; i++) {
    const unwrapFee = (await publicClient.readContract({
      address: liquidityManagerAddress,
      abi: liquidityManagerAbi,
      functionName: 'quoteUnwrapFee',
      args: [candidate],
    })) as bigint;

    const netOut = candidate - unwrapFee;
    if (netOut >= gelatoFee) {
      // Add buffer
      const withBuffer = candidate + (candidate * RELAY_FEE_BUFFER_BPS) / BPS_DENOMINATOR;
      return {
        relayerFee: withBuffer,
        gelatoFee,
        unwrapFee,
      };
    }
    // Increase candidate
    candidate = gelatoFee + unwrapFee + 1n;
  }

  // Fallback: return with generous buffer
  const finalUnwrapFee = (await publicClient.readContract({
    address: liquidityManagerAddress,
    abi: liquidityManagerAbi,
    functionName: 'quoteUnwrapFee',
    args: [candidate],
  })) as bigint;

  const withBuffer = candidate + (candidate * RELAY_FEE_BUFFER_BPS) / BPS_DENOMINATOR;
  return {
    relayerFee: withBuffer,
    gelatoFee,
    unwrapFee: finalUnwrapFee,
  };
}

/**
 * Encodes a relayTeleport calldata for the GelatoRelay contract.
 */
export function encodeRelayTeleport(params: {
  isGlobal: boolean;
  rootHint: bigint;
  gr: { chainId: bigint; recipient: Hex; tweak: Hex };
  proof: Hex;
  feeAuth: { relayerFee: bigint; maxFee: bigint; deadline: bigint; signature: Hex };
  maxGelatoFee: bigint;
}): Hex {
  return encodeFunctionData({
    abi: gelatoRelayAbi,
    functionName: 'relayTeleport',
    args: [
      params.isGlobal,
      params.rootHint,
      params.gr,
      params.proof,
      params.feeAuth,
      params.maxGelatoFee,
    ],
  });
}

/**
 * Encodes a relaySingleTeleport calldata for the GelatoRelay contract.
 */
export function encodeRelaySingleTeleport(params: {
  isGlobal: boolean;
  rootHint: bigint;
  gr: { chainId: bigint; recipient: Hex; tweak: Hex };
  proof: Hex;
  feeAuth: { relayerFee: bigint; maxFee: bigint; deadline: bigint; signature: Hex };
  maxGelatoFee: bigint;
}): Hex {
  return encodeFunctionData({
    abi: gelatoRelayAbi,
    functionName: 'relaySingleTeleport',
    args: [
      params.isGlobal,
      params.rootHint,
      params.gr,
      params.proof,
      params.feeAuth,
      params.maxGelatoFee,
    ],
  });
}

/**
 * Encodes a relayUnwrap calldata for the GelatoRelay contract.
 */
export function encodeRelayUnwrap(params: {
  owner: Address;
  amount: bigint;
  receiver: Address;
  deadline: bigint;
  v: number;
  r: Hex;
  s: Hex;
  maxGelatoFee: bigint;
}): Hex {
  return encodeFunctionData({
    abi: gelatoRelayAbi,
    functionName: 'relayUnwrap',
    args: [
      params.owner,
      params.amount,
      params.receiver,
      params.deadline,
      params.v,
      params.r,
      params.s,
      params.maxGelatoFee,
    ],
  });
}

/**
 * Encodes a relayTransfer calldata for the GelatoRelay contract.
 */
export function encodeRelayTransfer(params: {
  owner: Address;
  to: Address;
  amount: bigint;
  relayerFee: bigint;
  deadline: bigint;
  v: number;
  r: Hex;
  s: Hex;
  maxGelatoFee: bigint;
}): Hex {
  return encodeFunctionData({
    abi: gelatoRelayAbi,
    functionName: 'relayTransfer',
    args: [
      params.owner,
      params.to,
      params.amount,
      params.relayerFee,
      params.deadline,
      params.v,
      params.r,
      params.s,
      params.maxGelatoFee,
    ],
  });
}

/**
 * Submits a relay transaction via Gelato's callWithSyncFee.
 */
export async function submitTeleportRelay(params: SubmitTeleportRelayParams): Promise<SubmitTeleportRelayResult> {
  const { chainId, gelatoRelayAddress, feeToken, calldata, apiKey, gasLimit } = params;

  const relay = new GelatoRelay();
  const request: CallWithSyncFeeRequest = {
    chainId: BigInt(chainId),
    target: gelatoRelayAddress,
    data: calldata,
    feeToken,
    isRelayContext: true,
  };

  const options = gasLimit ? { gasLimit: gasLimit + GELATO_RELAY_GAS_OVERHEAD } : undefined;

  const response = await relay.callWithSyncFee(request, options, apiKey);
  return { taskId: response.taskId };
}

/**
 * Polls Gelato for task status until it reaches a terminal state or times out.
 */
export async function waitForRelayTask(
  taskId: string,
  options?: WaitForRelayTaskOptions,
): Promise<RelayTaskResult> {
  const polls = options?.polls ?? DEFAULT_POLLS;
  const intervalMs = options?.intervalMs ?? DEFAULT_INTERVAL_MS;

  const relay = new GelatoRelay();

  for (let i = 0; i < polls; i++) {
    const status = await relay.getTaskStatus(taskId);
    if (status) {
      const result: RelayTaskResult = {
        taskId: status.taskId,
        taskState: status.taskState as unknown as RelayTaskState,
        transactionHash: status.transactionHash,
        blockNumber: status.blockNumber,
        executionDate: status.executionDate,
        lastCheckMessage: status.lastCheckMessage,
      };

      if (
        result.taskState === RelayTaskState.ExecSuccess ||
        result.taskState === RelayTaskState.ExecReverted ||
        result.taskState === RelayTaskState.Cancelled
      ) {
        return result;
      }
    }

    if (i < polls - 1) {
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  return {
    taskId,
    taskState: RelayTaskState.CheckPending,
    lastCheckMessage: `Polling timed out after ${polls} attempts`,
  };
}
