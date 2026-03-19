import type { RelayFeeEstimate, RelayResult, RelayTeleportParams } from './types.js';

/**
 * Submit a teleport request to the custom relay node.
 */
export async function submitRelayTeleport(
  relayUrl: string,
  params: RelayTeleportParams,
): Promise<RelayResult> {
  const url = `${relayUrl.replace(/\/$/, '')}/relay/teleport`;

  const body = {
    is_global: params.isGlobal,
    root_hint: params.rootHint.toString(),
    chain_id: params.chainId.toString(),
    recipient: params.recipient,
    tweak: params.tweak,
    proof: params.proof,
    relayer_fee: params.relayerFee.toString(),
    max_fee: params.maxFee.toString(),
    deadline: params.deadline.toString(),
    signature: params.signature,
  };

  const response = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
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
  const url = `${relayUrl.replace(/\/$/, '')}/relay/fee-estimate?chain_id=${chainId}`;

  const response = await fetch(url);

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Relay node fee-estimate returned ${response.status}: ${text}`);
  }

  const result = (await response.json()) as { relayerFee: string };
  return { relayerFee: BigInt(result.relayerFee) };
}
