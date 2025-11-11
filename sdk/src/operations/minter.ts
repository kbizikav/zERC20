import type { PublicClient, WalletClient } from 'viem';
import { zeroAddress } from 'viem';
import { waitForTransactionReceipt } from 'viem/actions';

import { getMinterContract, getZerc20Contract } from '../onchain/contracts.js';
import { normalizeHex, toBigInt } from '../utils/hex.js';

export interface MinterDepositResult {
  transactionHash: string;
  approvalTransactionHash?: string;
}

export interface MinterWithdrawResult {
  transactionHash: string;
}

export interface MinterActionParams {
  walletClient: WalletClient;
  publicClient?: PublicClient;
  minterAddress: string;
  tokenAddress: string;
  amount: bigint;
}

function isZeroAddress(value: string): boolean {
  return BigInt(normalizeHex(value)) === BigInt(normalizeHex(zeroAddress));
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

export async function depositWithMinter({
  walletClient,
  publicClient,
  minterAddress,
  tokenAddress,
  amount,
}: MinterActionParams): Promise<MinterDepositResult> {
  const normalizedMinter = normalizeHex(minterAddress);
  const minter = getMinterContract(normalizedMinter, walletClient);
  const normalizedToken = normalizeHex(tokenAddress);
  const account = ensureAccount(walletClient);
  const receiptClientInstance = receiptClient(walletClient, publicClient);

  if (isZeroAddress(normalizedToken)) {
    const hash = await minter.write.depositNative([], {
      account,
      value: amount,
    });
    const receipt = await waitForTransactionReceipt(receiptClientInstance, { hash });
    return {
      transactionHash: receipt.transactionHash,
    };
  }

  const token = getZerc20Contract(normalizedToken, walletClient);
  const currentAllowance = ensureBigintLike(await token.read.allowance([account, normalizedMinter]), 'allowance');
  let approvalTransactionHash: string | undefined;

  if (currentAllowance < amount) {
    const approvalHash = await token.write.approve([normalizedMinter as `0x${string}`, amount], {
      account,
    });
    const approvalReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: approvalHash });
    approvalTransactionHash = approvalReceipt.transactionHash;
  }

  const depositHash = await minter.write.depositToken([amount], {
    account,
  });
  const depositReceipt = await waitForTransactionReceipt(receiptClientInstance, { hash: depositHash });
  return {
    transactionHash: depositReceipt.transactionHash,
    approvalTransactionHash,
  };
}

export async function withdrawWithMinter({
  walletClient,
  publicClient,
  minterAddress,
  tokenAddress,
  amount,
}: MinterActionParams): Promise<MinterWithdrawResult> {
  const normalizedMinter = normalizeHex(minterAddress);
  const minter = getMinterContract(normalizedMinter, walletClient);
  const normalizedToken = normalizeHex(tokenAddress);
  const account = ensureAccount(walletClient);
  const receiptClientInstance = receiptClient(walletClient, publicClient);

  const hash = isZeroAddress(normalizedToken)
    ? await minter.write.withdrawNative([amount], { account })
    : await minter.write.withdrawToken([amount], { account });
  const receipt = await waitForTransactionReceipt(receiptClientInstance, { hash });

  return {
    transactionHash: receipt.transactionHash,
  };
}
