import { useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { FileOperationStore } from './fileOperationStore';

import { FileOperationContext as Context } from './fileOperationContext';
export function FileOperationBoundary({ children }: { children: ReactNode }) {
  const [store] = useState(() => new FileOperationStore());
  useEffect(() => {
    store.activate();
    const leave = (event: BeforeUnloadEvent) => {
      if (store.hasPending()) {
        event.preventDefault();
        event.returnValue = '';
      }
    };
    window.addEventListener('beforeunload', leave);
    return () => {
      window.removeEventListener('beforeunload', leave);
      store.destroy();
    };
  }, [store]);
  return <Context.Provider value={store}>{children}</Context.Provider>;
}
export function FileOperationScopeBoundary({
  scope,
  children,
}: {
  scope: string | null;
  children: ReactNode;
}) {
  const root = useContext(Context);
  const store = useMemo(() => root?.forScope(scope) ?? null, [root, scope]);
  return <Context.Provider value={store}>{children}</Context.Provider>;
}
