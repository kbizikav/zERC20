// SPDX-License-Identifier: BUSL-1.1

import type {
  RelayFeeEstimate,
  RelayResult,
  RelaySwapParams,
  RelayTeleportParams,
  SwapQuote,
  SwapResult,
} from "./types.js";

/**
 * Submit a teleport request to the custom relay node.
 */
export async function submitRelayTeleport(
  relayUrl: string,
  params: RelayTeleportParams,
): Promise<RelayResult> {
  const url = `${relayUrl.replace(/\/$/, "")}/relay/teleport`;

  const body = {
    is_single: params.isSingle,
    is_global: params.isGlobal,
    root_hint: Number(params.rootHint),
    chain_id: Number(params.chainId),
    recipient: params.recipient,
    tweak: params.tweak,
    proof: params.proof,
    relayer_fee: params.relayerFee.toString(),
    max_fee: params.maxFee.toString(),
    deadline: Number(params.deadline),
    signature: params.signature,
  };

  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Relay node returned ${response.status}: ${text}`);
  }

  const result = (await response.json()) as { txHash: string };
  return { txHash: result.txHash };
}

/**
 * Fetch a fee estimate from the custom relay node.
 */
export async function estimateRelayFee(
  relayUrl: string,
  chainId: number,
): Promise<RelayFeeEstimate> {
  const url = `${relayUrl.replace(/\/$/, "")}/relay/fee-estimate?chain_id=${chainId}`;

  const response = await fetch(url);

  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Relay node fee-estimate returned ${response.status}: ${text}`,
    );
  }

  const result = (await response.json()) as { relayerFee: string };
  return { relayerFee: BigInt(result.relayerFee) };
}

/**
 * Fetch a swap quote from the relay node.
 */
export async function fetchSwapQuote(
  relayUrl: string,
  chainId: number,
  amount: bigint,
): Promise<SwapQuote> {
  const url = `${relayUrl.replace(/\/$/, "")}/relay/swap-quote?chain_id=${chainId}&amount=${amount.toString()}`;

  const response = await fetch(url);

  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Relay node swap-quote returned ${response.status}: ${text}`,
    );
  }

  const result = (await response.json()) as {
    nativeAmount: string;
    feeBps: number;
    relayerFee: string;
  };
  return {
    nativeAmount: BigInt(result.nativeAmount),
    feeBps: result.feeBps,
    relayerFee: BigInt(result.relayerFee),
  };
}

/**
 * Submit a token-to-native swap request to the relay node.
 */
export async function submitRelaySwap(
  relayUrl: string,
  params: RelaySwapParams,
): Promise<SwapResult> {
  const url = `${relayUrl.replace(/\/$/, "")}/relay/swap`;

  const body = {
    chainId: params.chainId,
    tokenAmount: params.tokenAmount.toString(),
    minNativeAmount: params.minNativeAmount.toString(),
    recipient: params.recipient,
    owner: params.owner,
    permitDeadline: params.permitDeadline.toString(),
    permitV: params.permitV,
    permitR: params.permitR,
    permitS: params.permitS,
  };

  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Relay node swap returned ${response.status}: ${text}`);
  }

  const result = (await response.json()) as { txHash: string };
  return { txHash: result.txHash };
}
