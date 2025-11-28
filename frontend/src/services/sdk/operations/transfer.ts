import { BigNumberish, Signer } from 'ethers';

import { getZerc20Contract } from '../onchain/contracts.js';
import { normalizeHex } from '../core/utils.js';

export interface TransferResult {
  transactionHash: string;
}

export async function submitTransfer(
  signer: Signer,
  tokenAddress: string,
  to: string,
  amount: BigNumberish,
): Promise<TransferResult> {
  const contract = getZerc20Contract(normalizeHex(tokenAddress), signer);
  const tx = await contract.transfer(normalizeHex(to), amount);
  const response = await tx.wait();
  return {
    transactionHash: response?.hash ?? tx.hash,
  };
}
