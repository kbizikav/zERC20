import { PropsWithChildren, createContext, useContext, useMemo } from 'react';
import { RuntimeConfig, resolveRuntimeConfig } from './appConfig';

const ConfigContext = createContext<RuntimeConfig | null>(null);

export function ConfigProvider({ children }: PropsWithChildren): JSX.Element {
  const value = useMemo<RuntimeConfig>(() => resolveRuntimeConfig(import.meta.env), []);
  return <ConfigContext.Provider value={value}>{children}</ConfigContext.Provider>;
}

export function useRuntimeConfig(): RuntimeConfig {
  const ctx = useContext(ConfigContext);
  if (!ctx) {
    throw new Error('useRuntimeConfig must be used within ConfigProvider');
  }
  return ctx;
}
