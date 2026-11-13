import {
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
} from './live/autoRefresh';
import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { ApiError } from './sessionHttp';
import { readChanges, markChangesSeen } from './changesApi';
import type { ChangesPage } from './changesModel';
interface State {
  page: ChangesPage | null;
  loading: boolean;
  marking: boolean;
  error: string;
  expired: boolean;
  unavailable: boolean;
  message: string;
}
const initial: State = {
  page: null,
  loading: true,
  marking: false,
  error: '',
  expired: false,
  unavailable: false,
  message: '',
};
const message = (error: unknown) =>
  error instanceof Error ? error.message : '变更读取失败，请重试。';
export function useChanges(project: string | null, active = true) {
  const [state, setState] = useState(initial);
  const revision = useRef(0);
  const latest = useRef(state);
  useLayoutEffect(() => {
    latest.current = state;
  });
  const read = useRef<AbortController | null>(null);
  const write = useRef<AbortController | null>(null);
  const pending = useRef<{ snapshot: string; id: string } | null>(null);
  const load = useCallback(
    async (before: string | null, previous: ChangesPage | null = null) => {
      revision.current += 1;
      read.current?.abort();
      const controller = new AbortController();
      read.current = controller;
      setState({ ...initial });
      try {
        const page = await readChanges(
          project,
          before,
          controller.signal,
          previous,
        );
        if (!controller.signal.aborted)
          setState({ ...initial, page, loading: false });
      } catch (error) {
        if (!controller.signal.aborted)
          setState({
            ...initial,
            loading: false,
            error: message(error),
            expired: error instanceof ApiError && error.status === 409,
            unavailable:
              error instanceof ApiError &&
              [404, 405, 501].includes(error.status),
          });
      }
    },
    [project],
  );
  useLayoutEffect(() => {
    if (active) void load(null);
    return () => {
      revision.current += 1;
      read.current?.abort();
      write.current?.abort();
    };
  }, [load, active]);
  const reload = useCallback(() => load(null), [load]);
  const revalidate = useCallback<Revalidate>(
    async (signal) => {
      if (
        latest.current.loading ||
        latest.current.marking ||
        latest.current.unavailable ||
        write.current !== null
      )
        return REVALIDATED;
      const started = revision.current;
      try {
        const page = await readChanges(project, null, signal);
        if (!signal.aborted && started === revision.current)
          setState({ ...initial, page, loading: false });
        return REVALIDATED;
      } catch (error) {
        if (signal.aborted || started !== revision.current) return REVALIDATED;
        return revalidationFailed(message(error));
      }
    },
    [project],
  );
  const mark = async () => {
    if (!state.page || state.loading || state.marking) return;
    revision.current += 1;
    const page = state.page;
    if (pending.current?.snapshot !== page.snapshot)
      pending.current = { snapshot: page.snapshot, id: crypto.randomUUID() };
    const request = new AbortController();
    write.current = request;
    setState((s) => ({ ...s, marking: true, error: '', message: '' }));
    try {
      await markChangesSeen(page, pending.current.id, request.signal);
      if (request.signal.aborted) return;
      pending.current = null;
      await load(null);
      if (!request.signal.aborted)
        setState((s) => ({ ...s, message: '此快照内的变化已标记为已读。' }));
    } catch (error) {
      if (!request.signal.aborted)
        setState((s) => ({
          ...s,
          marking: false,
          error: message(error),
          expired: error instanceof ApiError && error.status === 409,
        }));
    } finally {
      if (write.current === request) write.current = null;
    }
  };
  return {
    ...state,
    reload,
    revalidate,
    next: () =>
      state.page?.next_cursor
        ? load(state.page.next_cursor, state.page)
        : Promise.resolve(),
    mark,
  };
}
