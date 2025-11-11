import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

export interface StoredAnnouncement {
  id: string;
  burnAddress: string;
  fullBurnAddress: string;
  createdAtNs: string;
  recipientChainId: string;
}

interface StorageDataState {
  seeds: Record<string, string>;
  invoices: Record<string, string[]>;
  vetKeys: Record<string, string>;
  announcements: Record<string, StoredAnnouncement[]>;
}

interface StorageActions {
  setSeed: (account: string, seed: string) => void;
  removeSeed: (account: string) => void;
  setInvoices: (account: string, invoices: string[]) => void;
  removeInvoices: (account: string) => void;
  setVetKey: (account: string, vetKeyHex: string) => void;
  removeVetKey: (account: string) => void;
  setAnnouncements: (account: string, announcements: StoredAnnouncement[]) => void;
  removeAnnouncements: (account: string) => void;
  clearAll: () => void;
}

export type StorageState = StorageDataState & StorageActions;

const createInitialDataState = (): StorageDataState => ({
  seeds: {},
  invoices: {},
  vetKeys: {},
  announcements: {},
});

const noopStorage: Storage = {
  getItem: () => null,
  setItem: () => undefined,
  removeItem: () => undefined,
  clear: () => undefined,
  key: () => null,
  get length() {
    return 0;
  },
};

export const useStorageStore = create<StorageState>()(
  persist(
    (set) => ({
      ...createInitialDataState(),
      setSeed: (account, seed) =>
        set((state) => ({
          seeds: { ...state.seeds, [account]: seed },
        })),
      removeSeed: (account) =>
        set((state) => {
          if (!(account in state.seeds)) {
            return state;
          }
          const next = { ...state.seeds };
          delete next[account];
          return { seeds: next };
        }),
      setInvoices: (account, invoices) =>
        set((state) => ({
          invoices: { ...state.invoices, [account]: invoices },
        })),
      removeInvoices: (account) =>
        set((state) => {
          if (!(account in state.invoices)) {
            return state;
          }
          const next = { ...state.invoices };
          delete next[account];
          return { invoices: next };
        }),
      setVetKey: (account, vetKeyHex) =>
        set((state) => ({
          vetKeys: { ...state.vetKeys, [account]: vetKeyHex },
        })),
      removeVetKey: (account) =>
        set((state) => {
          if (!(account in state.vetKeys)) {
            return state;
          }
          const next = { ...state.vetKeys };
          delete next[account];
          return { vetKeys: next };
        }),
      setAnnouncements: (account, announcements) =>
        set((state) => ({
          announcements: { ...state.announcements, [account]: announcements },
        })),
      removeAnnouncements: (account) =>
        set((state) => {
          if (!(account in state.announcements)) {
            return state;
          }
          const next = { ...state.announcements };
          delete next[account];
          return { announcements: next };
        }),
      clearAll: () => set(() => createInitialDataState()),
    }),
    {
      name: 'zerc20-storage',
      storage: createJSONStorage(() =>
        typeof window === 'undefined' ? noopStorage : window.localStorage,
      ),
      version: 1,
    },
  ),
);
