import type { ReactNode } from 'react';
import { getExplorerTxUrl } from '@utils/explorer';

type SupportedChainId = bigint | number | string;

interface TeleportMessageParams {
  label: string;
  txHash: string;
  chainId: SupportedChainId;
}

export function createTeleportSubmittedMessage({
  label,
  txHash,
  chainId,
}: TeleportMessageParams): ReactNode {
  const explorerUrl = getExplorerTxUrl(chainId, txHash);

  if (!explorerUrl) {
    return `${label}: ${txHash}`;
  }

  return (
    <>
      {label}:{' '}
      <a className="mono" href={explorerUrl} target="_blank" rel="noopener noreferrer">
        {txHash}
      </a>
    </>
  );
}
