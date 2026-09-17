import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { DraftStore } from './draftStore';
import { DraftContext, useDraftStore } from './draftContext';

export function DraftBoundary({ children }: { children: ReactNode }) {
  const [store] = useState(() => new DraftStore());
  useEffect(() => {
    store.activate();
    const leave = (event: BeforeUnloadEvent) => {
      if (store.hasAny()) {
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
  return (
    <DraftContext.Provider value={store}>{children}</DraftContext.Provider>
  );
}
export function DraftScopeBoundary({
  scope,
  children,
}: {
  scope: string | null;
  children: ReactNode;
}) {
  const root = useDraftStore();
  const scoped = useMemo(() => root.forScope(scope), [root, scope]);
  return (
    <DraftContext.Provider value={scoped}>{children}</DraftContext.Provider>
  );
}
