import { useEffect, useState } from 'react';
import { ApiError, errorMessage, getProjectOperations } from './api';
import type { ProjectOperationsData } from './projectOperations';

interface OperationsState {
  key: string;
  result: ProjectOperationsData | null;
  error: string | null;
  stale: boolean;
}

export function useProjectOperations(project: string, generation: number) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<OperationsState | null>(null);
  const key = JSON.stringify([project, generation, attempt]);

  useEffect(() => {
    const controller = new AbortController();
    getProjectOperations(project, generation, controller.signal)
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
  }, [project, generation, attempt, key]);

  const current = state?.key === key ? state : null;
  return {
    result: current?.result ?? null,
    error: current?.error ?? null,
    stale: current?.stale ?? false,
    loading: current === null,
    retry: () => setAttempt((value) => value + 1),
  };
}
