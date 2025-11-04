import { PropsWithChildren } from 'react';
import { ConfigProvider } from '@config/ConfigContext';
import { WalletProvider } from './WalletProvider';

export function AppProviders({ children }: PropsWithChildren): JSX.Element {
  return (
    <ConfigProvider>
      <WalletProvider>{children}</WalletProvider>
    </ConfigProvider>
  );
}
