import { useCallback, useEffect, useRef, useState } from 'react';
import { errorMessage, getWorkbench } from './api';
import type { Workbench } from './types';

export function useWorkbench() {
  const [data, setData] = useState<Workbench | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef<AbortController | null>(null);
  const sequence = useRef(0);

  const load = useCallback(async (refresh = false) => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const current = ++sequence.current;
    setLoading(true);
    setError(null);
    try {
      const next = await getWorkbench(controller.signal, refresh);
      if (current === sequence.current && !controller.signal.aborted) {
        setData(next);
        return next;
      }
    } catch (cause) {
      if (current === sequence.current && !controller.signal.aborted) {
        setError(errorMessage(cause));
      }
    } finally {
      if (current === sequence.current && !controller.signal.aborted) {
        setLoading(false);
      }
    }
    return null;
  }, []);

  useEffect(() => {
    void load();
    return () => request.current?.abort();
  }, [load]);

  const refreshLatest = useCallback(() => load(true), [load]);
  return {
    data,
    loading,
    error,
    refresh: () => void load(true),
    refreshLatest,
  };
}
