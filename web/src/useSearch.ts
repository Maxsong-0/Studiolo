import { useEffect, useRef, useState } from 'react';
import { ApiError, errorMessage, getSearch } from './api';
import { queryError } from './searchQuery';
import type { SearchMode, SearchResponse } from './types';

interface SearchState {
  key: string;
  result: SearchResponse | null;
  error: string | null;
  status: number;
  loading: boolean;
}

export function useSearch(
  query: string,
  mode: SearchMode,
  project: string,
  generation: number,
) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<SearchState | null>(null);
  const sequence = useRef(0);
  const normalized = query.trim();
  const invalid = queryError(query);
  const key = JSON.stringify([
    normalized,
    mode,
    project,
    generation,
    attempt,
    invalid,
  ]);

  useEffect(() => {
    if (invalid) return;
    const current = ++sequence.current;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      setState({ key, result: null, error: null, status: 0, loading: true });
      getSearch(normalized, mode, project, generation, controller.signal)
        .then((result) => {
          if (!controller.signal.aborted && current === sequence.current) {
            setState({ key, result, error: null, status: 0, loading: false });
          }
        })
        .catch((cause: unknown) => {
          if (!controller.signal.aborted && current === sequence.current) {
            setState({
              key,
              result: null,
              error: errorMessage(cause),
              status: cause instanceof ApiError ? cause.status : 0,
              loading: false,
            });
          }
        });
    }, 200);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [normalized, mode, project, generation, attempt, invalid, key]);

  const current = state?.key === key ? state : null;
  return {
    result: invalid ? null : (current?.result ?? null),
    error: invalid ?? current?.error ?? null,
    status: current?.status ?? 0,
    loading: !invalid && (current?.loading ?? true),
    invalid: Boolean(invalid),
    retry: () => setAttempt((value) => value + 1),
  };
}
