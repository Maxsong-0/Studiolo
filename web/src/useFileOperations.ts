import { useEffect, useState } from 'react';
import { discoverFileOperations } from './fileOperationApi';
import { useFileOperationStore } from './fileOperationContext';
import { ApiError } from './sessionHttp';

export function useFileOperations() {
  const { store } = useFileOperationStore();
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    scope: string | null;
    vault: string | null;
    rename: boolean;
    attempt: number;
    phase: 'loading' | 'ready' | 'disabled' | 'error';
  }>({
    scope: null,
    vault: null,
    rename: false,
    attempt: -1,
    phase: 'loading',
  });
  useEffect(() => {
    const controller = new AbortController();
    if (!store.ownerScope) return () => controller.abort();
    void discoverFileOperations(controller.signal).then(
      (capability) => {
        if (!controller.signal.aborted)
          setState({
            scope: store.ownerScope,
            vault: capability.vault_uuid,
            rename: capability.rename_enabled,
            attempt,
            phase: 'ready',
          });
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        const disabled =
          error instanceof ApiError &&
          error.status === 403 &&
          error.details &&
          typeof error.details === 'object' &&
          'code' in error.details &&
          error.details.code === 'disabled';
        setState({
          scope: store.ownerScope,
          vault: null,
          rename: false,
          attempt,
          phase: disabled ? 'disabled' : 'error',
        });
      },
    );
    return () => controller.abort();
  }, [store.ownerScope, attempt]);
  const current = state.scope === store.ownerScope && state.attempt === attempt;
  return {
    vault: current ? state.vault : null,
    renameEnabled: current && state.phase === 'ready' && state.rename,
    phase: !store.ownerScope ? 'disabled' : current ? state.phase : 'loading',
    retry: () => setAttempt((n) => n + 1),
  };
}
