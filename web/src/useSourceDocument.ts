import { useEffect, useState } from 'react';
import { ApiError, errorMessage, getDocument } from './api';
import type { SourceDocument } from './types';

interface DocumentState {
  key: string;
  document: SourceDocument | null;
  error: string | null;
  stale: boolean;
}

export function useSourceDocument(path: string, generation: number) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<DocumentState | null>(null);
  const key = JSON.stringify([path, generation, attempt]);
  useEffect(() => {
    const controller = new AbortController();
    getDocument(path, generation, controller.signal)
      .then((document) => {
        if (!controller.signal.aborted)
          setState({ key, document, error: null, stale: false });
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted)
          setState({
            key,
            document: null,
            error: errorMessage(cause),
            stale: cause instanceof ApiError && cause.status === 409,
          });
      });
    return () => controller.abort();
  }, [path, generation, attempt, key]);
  const current = state?.key === key ? state : null;
  return {
    document: current?.document ?? null,
    error: current?.error ?? null,
    stale: current?.stale ?? false,
    retry: () => setAttempt((value) => value + 1),
  };
}
