import { Signer, ZeroAddress } from 'ethers';

import { getMinterContract, getZerc20Contract } from '../onchain/contracts.js';
import { normalizeHex } from '../core/utils.js';

export interface MinterDepositResult {
  transactionHash: string;
  approvalTransactionHash?: string;
}

export interface MinterWithdrawResult {
  transactionHash: string;
}

export interface MinterActionParams {
  signer: Signer;
  minterAddress: string;
  tokenAddress: string;
  amount: bigint;
}

function isZeroAddress(value: string): boolean {
  return BigInt(normalizeHex(value)) === BigInt(normalizeHex(ZeroAddress));
}

export async function depositWithMinter({
  signer,
  minterAddress,
  tokenAddress,
  amount,
}: MinterActionParams): Promise<MinterDepositResult> {
  const normalizedMinter = normalizeHex(minterAddress);
  const minter = getMinterContract(normalizedMinter, signer);
  const normalizedToken = normalizeHex(tokenAddress);

  if (isZeroAddress(normalizedToken)) {
    const tx = await minter.depositNative({ value: amount });
    const receipt = await tx.wait();
    return {
      transactionHash: receipt?.hash ?? tx.hash,
    };
  }

  const token = getZerc20Contract(normalizedToken, signer);
  const owner = await signer.getAddress();
  const currentAllowance = await token.allowance(owner, normalizedMinter);
  let approvalTransactionHash: string | undefined;

  if (currentAllowance < amount) {
    const approvalTx = await token.approve(normalizedMinter, amount);
    const approvalReceipt = await approvalTx.wait();
    approvalTransactionHash = approvalReceipt?.hash ?? approvalTx.hash;
  }

  const depositTx = await minter.depositToken(amount);
  const depositReceipt = await depositTx.wait();
  return {
    transactionHash: depositReceipt?.hash ?? depositTx.hash,
    approvalTransactionHash,
  };
}

export async function withdrawWithMinter({
  signer,
  minterAddress,
  tokenAddress,
  amount,
}: MinterActionParams): Promise<MinterWithdrawResult> {
  const normalizedMinter = normalizeHex(minterAddress);
  const minter = getMinterContract(normalizedMinter, signer);
  const normalizedToken = normalizeHex(tokenAddress);

  const withdrawTx = isZeroAddress(normalizedToken)
    ? await minter.withdrawNative(amount)
    : await minter.withdrawToken(amount);
  const withdrawReceipt = await withdrawTx.wait();

  return {
    transactionHash: withdrawReceipt?.hash ?? withdrawTx.hash,
  };
}
