import type { PublicClient, WalletClient } from 'viem';
import { zeroAddress } from 'viem';
import { waitForTransactionReceipt } from 'viem/actions';

import { NATIVE_TOKEN } from '../constants.js';
import { getErc20Contract, getLiquidityManagerContract } from '../onchain/contracts.js';
import { normalizeHex, toBigInt } from '../utils/hex.js';

export interface LiquidityWrapResult {
  transactionHash: string;
  approvalTransactionHash?: string;
}

export interface LiquidityUnwrapResult {
  transactionHash: string;
}

export interface LiquidityActionParams {
  walletClient: WalletClient;
  publicClient?: PublicClient;
  liquidityManagerAddress: string;
  amount: bigint;
  receiver?: string;
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

function toAddress(value?: string): `0x${string}` {
  if (!value) {
    throw new Error('missing address value');
  }
  return normalizeHex(value) as `0x${string}`;
}

export async function wrapWithLiquidityManager({
  walletClient,
  publicClient,
  liquidityManagerAddress,
  amount,
  receiver,
}: LiquidityActionParams): Promise<LiquidityWrapResult> {
  const normalizedManager = normalizeHex(liquidityManagerAddress) as `0x${string}`;
  const manager = getLiquidityManagerContract(normalizedManager, walletClient);
  const account = ensureAccount(walletClient);
  const chain = walletClient.chain;
  const receiptClientInstance = receiptClient(walletClient, publicClient);
  const receiverAddress = (receiver ? normalizeHex(receiver) : account) as `0x${string}`;

  const underlying = toAddress((await manager.read.underlyingToken()) as string);
  if (underlying === normalizeHex(zeroAddress)) {
    throw new Error('liquidity manager is not configured with an underlying token');
  }
  const nativeToken = normalizeHex(NATIVE_TOKEN) as `0x${string}`;
  const isNative = underlying === nativeToken;
  let approvalTransactionHash: string | undefined;

  if (!isNative) {
    const underlyingToken = getErc20Contract(underlying, walletClient);
    const currentAllowance = ensureBigintLike(
      await underlyingToken.read.allowance([account, normalizedManager]),
      'allowance',
    );

    if (currentAllowance < amount) {
      const approvalHash = await underlyingToken.write.approve([normalizedManager as `0x${string}`, amount], {
        account,
        chain,
      });
      const approvalReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: approvalHash });
      approvalTransactionHash = approvalReceipt.transactionHash;
    }
  }

  const wrapHash = await manager.write.wrap([amount, receiverAddress], {
    account,
    chain,
    value: isNative ? amount : undefined,
  });
  const wrapReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: wrapHash });
  return {
    transactionHash: wrapReceipt.transactionHash,
    approvalTransactionHash,
  };
}

export async function unwrapWithLiquidityManager({
  walletClient,
  publicClient,
  liquidityManagerAddress,
  amount,
  receiver,
}: LiquidityActionParams): Promise<LiquidityUnwrapResult> {
  const normalizedManager = normalizeHex(liquidityManagerAddress) as `0x${string}`;
  const manager = getLiquidityManagerContract(normalizedManager, walletClient);
  const account = ensureAccount(walletClient);
  const chain = walletClient.chain;
  const receiptClientInstance = receiptClient(walletClient, publicClient);
  const receiverAddress = (receiver ? normalizeHex(receiver) : account) as `0x${string}`;

  const unwrapHash = await manager.write.unwrap([amount, receiverAddress], { account, chain });
  const receipt = await waitForTransactionReceipt(receiptClientInstance, { hash: unwrapHash });

  return {
    transactionHash: receipt.transactionHash,
  };
}
