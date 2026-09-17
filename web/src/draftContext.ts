import {
  createContext,
  useContext,
  useState,
  useSyncExternalStore,
} from 'react';
import { DraftStore } from './draftStore';

export const DraftContext = createContext<DraftStore | null>(null);
export function useDraftStore() {
  const supplied = useContext(DraftContext);
  const [fallback] = useState(() => new DraftStore());
  return supplied ?? fallback;
}
export function useDraftCount() {
  const store = useDraftStore();
  const count = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return { store, count };
}
