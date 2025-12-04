import { normalizeHex } from '@zerc20/sdk';

export function toAccountKey(account?: string): string | undefined {
  if (!account) {
    return undefined;
  }
  try {
    return normalizeHex(account);
  } catch {
    return account.toLowerCase();
  }
}
