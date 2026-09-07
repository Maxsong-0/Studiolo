import { useEffect, useState } from 'react';
import { ApiError, errorMessage, getTree } from './api';
import type { TreeResponse } from './treeTypes';

interface DirectoryState {
  key: string;
  result: TreeResponse | null;
  error: string | null;
  stale: boolean;
}

export function useDirectory(path: string, offset: number, generation: number) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<DirectoryState | null>(null);
  const key = JSON.stringify([path, offset, generation, attempt]);

  useEffect(() => {
    const controller = new AbortController();
    getTree(path, offset, generation, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted)
          setState({ key, result, error: null, stale: false });
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted)
          setState({
            key,
            result: null,
            error: errorMessage(cause),
            stale: cause instanceof ApiError && cause.status === 409,
          });
      });
    return () => controller.abort();
  }, [path, offset, generation, attempt, key]);

  const current = state?.key === key ? state : null;
  return {
    result: current?.result ?? null,
    error: current?.error ?? null,
    stale: current?.stale ?? false,
    loading: current === null,
    retry: () => setAttempt((value) => value + 1),
  };
}
