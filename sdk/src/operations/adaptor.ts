import type { PublicClient, WalletClient } from 'viem';
import { zeroAddress } from 'viem';
import { waitForTransactionReceipt } from 'viem/actions';

import { getAdaptorContract, getZerc20Contract } from '../onchain/contracts.js';
import { normalizeHex, toBigInt } from '../utils/hex.js';

type ContractClient = PublicClient | WalletClient;

export interface AdaptorFeeQuote {
  tokenUnwrapFee: bigint;
  nativeBridgeFee: bigint;
  tokenBridgeFee: bigint;
}

export interface AdaptorBridgeRequest {
  dstEid: number | bigint | string;
  extraOptions?: string;
  composeMsg?: string;
  oftCmd?: string;
  refundAddress?: string;
  to?: string;
  minAmountOut: bigint;
}

export interface AdaptorBridgeParams {
  walletClient: WalletClient;
  publicClient?: PublicClient;
  adaptorAddress: string;
  amount: bigint;
  request: AdaptorBridgeRequest;
}

export interface AdaptorBridgeResult {
  transactionHash: string;
  approvalTransactionHash?: string;
  feeQuote: AdaptorFeeQuote;
}

function ensureAccount(walletClient: WalletClient): `0x${string}` {
  const account = walletClient.account?.address;
  if (!account) {
    throw new Error('wallet client is missing default account');
  }
  return normalizeHex(account) as `0x${string}`;
}

function receiptClient(walletClient: WalletClient, publicClient?: PublicClient): PublicClient | WalletClient {
  return publicClient ?? walletClient;
}

function ensureBigintLike(value: unknown, label: string): bigint {
  if (typeof value === 'bigint' || typeof value === 'number' || typeof value === 'string') {
    return toBigInt(value);
  }
  throw new Error(`${label} must be bigint-like value`);
}

function normalizeBytes(value?: string): `0x${string}` {
  const normalized = value ? normalizeHex(value) : '0x';
  return normalized as `0x${string}`;
}

function normalizeBridgeRequest(request: AdaptorBridgeRequest, account: `0x${string}`) {
  const dstEid = ensureBigintLike(request.dstEid, 'dstEid');
  const refundAddress = request.refundAddress ? normalizeHex(request.refundAddress) : account;
  const to = request.to ? normalizeHex(request.to) : account;
  const minAmountOut = ensureBigintLike(request.minAmountOut, 'minAmountOut');
  return {
    dstEid,
    refundAddress,
    to,
    minAmountOut,
    extraOptions: normalizeBytes(request.extraOptions),
    composeMsg: normalizeBytes(request.composeMsg),
    oftCmd: normalizeBytes(request.oftCmd),
  };
}

export async function quoteAdaptorFee({
  publicClient,
  adaptorAddress,
  amount,
  request,
}: {
  publicClient: ContractClient;
  adaptorAddress: string;
  amount: bigint;
  request: AdaptorBridgeRequest;
}): Promise<AdaptorFeeQuote> {
  const adaptor = getAdaptorContract(adaptorAddress, publicClient);
  const normalizedRequest = normalizeBridgeRequest(request, normalizeHex(zeroAddress) as `0x${string}`);
  const fee = (await adaptor.read.quoteFee([amount, normalizedRequest])) as unknown as {
    tokenUnwrapFee: bigint;
    nativeBridgeFee: bigint;
    tokenBridgeFee: bigint;
  };
  return {
    tokenUnwrapFee: fee.tokenUnwrapFee,
    nativeBridgeFee: fee.nativeBridgeFee,
    tokenBridgeFee: fee.tokenBridgeFee,
  };
}

export async function unwrapAndBridgeWithAdaptor({
  walletClient,
  publicClient,
  adaptorAddress,
  amount,
  request,
}: AdaptorBridgeParams): Promise<AdaptorBridgeResult> {
  const normalizedAdaptor = normalizeHex(adaptorAddress);
  const adaptor = getAdaptorContract(normalizedAdaptor, walletClient);
  const account = ensureAccount(walletClient);
  const receiptClientInstance = receiptClient(walletClient, publicClient);

  const zerc20Address = normalizeHex((await adaptor.read.zerc20()) as string);
  const zerc20 = getZerc20Contract(zerc20Address, walletClient);
  const currentAllowance = ensureBigintLike(await zerc20.read.allowance([account, normalizedAdaptor]), 'allowance');
  let approvalTransactionHash: string | undefined;

  if (currentAllowance < amount) {
    const approvalHash = await zerc20.write.approve([normalizedAdaptor as `0x${string}`, amount], {
      account,
    });
    const approvalReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: approvalHash });
    approvalTransactionHash = approvalReceipt.transactionHash;
  }

  const normalizedRequest = normalizeBridgeRequest(request, account);
  const feeQuote = await quoteAdaptorFee({
    publicClient: (publicClient ?? walletClient) as ContractClient,
    adaptorAddress: normalizedAdaptor,
    amount,
    request: normalizedRequest,
  });

  const bridgeHash = await adaptor.write.unwrapAndBridge([amount, normalizedRequest], {
    account,
    value: feeQuote.nativeBridgeFee,
  });
  const bridgeReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: bridgeHash });

  return {
    transactionHash: bridgeReceipt.transactionHash,
    approvalTransactionHash,
    feeQuote,
  };
}
