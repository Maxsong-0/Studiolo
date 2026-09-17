import {
  createContext,
  useContext,
  useState,
  useSyncExternalStore,
} from 'react';
import { FileOperationStore } from './fileOperationStore';

export const FileOperationContext = createContext<FileOperationStore | null>(
  null,
);
export function useFileOperationStore() {
  const supplied = useContext(FileOperationContext);
  const [fallback] = useState(() => new FileOperationStore());
  const store = supplied ?? fallback;
  const record = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return { store, record };
}
