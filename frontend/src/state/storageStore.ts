import { create } from 'zustand';
import { createJSONStorage, persist, type StateStorage } from 'zustand/middleware';

export interface StoredAnnouncement {
  id: string;
  burnAddress: string;
  fullBurnAddress: string;
  createdAtNs: string;
  recipientChainId: string;
}

type StorageCollections = {
  seeds: Record<string, string>;
  invoices: Record<string, string[]>;
  vetKeys: Record<string, string>;
  announcements: Record<string, StoredAnnouncement[]>;
};

type StorageActions = {
  setSeed: (account: string, seed: string) => void;
  removeSeed: (account: string) => void;
  setInvoices: (account: string, invoices: string[]) => void;
  setVetKey: (account: string, vetKeyHex: string) => void;
  removeVetKey: (account: string) => void;
  setAnnouncements: (account: string, records: StoredAnnouncement[]) => void;
  clearAll: () => void;
};

export type StorageState = StorageCollections & StorageActions;

const createInitialCollections = (): StorageCollections => ({
  seeds: {},
  invoices: {},
  vetKeys: {},
  announcements: {},
});

const createMemoryStorage = (): StateStorage => {
  const store = new Map<string, string>();
  return {
    getItem: (name) => store.get(name) ?? null,
    removeItem: (name) => {
      store.delete(name);
    },
    setItem: (name, value) => {
      store.set(name, value);
    },
  };
};

const resolveStorage = (): StateStorage => {
  if (typeof window === 'undefined' || !window.localStorage) {
    return createMemoryStorage();
  }
  return window.localStorage;
};

const persistStorage = createJSONStorage<StorageState>(resolveStorage);

export const useStorageStore = create<StorageState>()(
  persist(
    (set) => ({
      ...createInitialCollections(),
      setSeed: (account, seed) => {
        set((state) => ({
          seeds: { ...state.seeds, [account]: seed },
        }));
      },
      removeSeed: (account) => {
        set((state) => {
          if (!(account in state.seeds)) {
            return state;
          }
          const next = { ...state.seeds };
          delete next[account];
          return { seeds: next };
        });
      },
      setInvoices: (account, invoices) => {
        set((state) => ({
          invoices: { ...state.invoices, [account]: [...invoices] },
        }));
      },
      setVetKey: (account, vetKeyHex) => {
        set((state) => ({
          vetKeys: { ...state.vetKeys, [account]: vetKeyHex },
        }));
      },
      removeVetKey: (account) => {
        set((state) => {
          if (!(account in state.vetKeys)) {
            return state;
          }
          const next = { ...state.vetKeys };
          delete next[account];
          return { vetKeys: next };
        });
      },
      setAnnouncements: (account, records) => {
        set((state) => ({
          announcements: { ...state.announcements, [account]: [...records] },
        }));
      },
      clearAll: () => {
        set(() => createInitialCollections());
        useStorageStore.persist?.clearStorage?.();
      },
    }),
    {
      name: 'zerc20-storage',
      version: 1,
      storage: persistStorage,
    },
  ),
);
