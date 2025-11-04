export function formatChainId(chainId: bigint): string {
  const decimal = chainId.toString(10);
  const hex = chainId.toString(16);
  return `${decimal} (0x${hex})`;
}
