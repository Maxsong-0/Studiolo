import {
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
} from './live/autoRefresh';
import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { ApiError } from './sessionHttp';
import { readChanges, markChangesSeen } from './changesApi';
import { readChangesHead, sameChanges } from './changesHead';
import type { ChangesPage } from './changesModel';
interface State {
  page: ChangesPage | null;
  loading: boolean;
  marking: boolean;
  error: string;
  expired: boolean;
  unavailable: boolean;
  message: string;
  /** 水位变了：别处有了新变化，手里这一页已经不是最新（T-210）。 */
  behind: boolean;
}
const initial: State = {
  page: null,
  loading: true,
  marking: false,
  error: '',
  expired: false,
  unavailable: false,
  message: '',
  behind: false,
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
  const headGone = useRef(false);
  /**
   * 只问水位（T-210）：变了就亮「有新变化」，不替人重读，免得翻到一半的列表被换掉。
   * 水位没变不读列表；已经亮着提示就不再问。
   */
  const watch = useCallback<Revalidate>(
    async (signal) => {
      const now = latest.current;
      const page = now.page;
      if (
        !active ||
        headGone.current ||
        !page ||
        now.loading ||
        now.marking ||
        now.behind ||
        write.current !== null
      )
        return REVALIDATED;
      const started = revision.current;
      try {
        const head = await readChangesHead(page.vault_uuid, signal);
        if (signal.aborted || started !== revision.current) return REVALIDATED;
        if (!sameChanges(head.through, page.through))
          setState((s) => (s.page === page ? { ...s, behind: true } : s));
        return REVALIDATED;
      } catch (error) {
        if (signal.aborted || started !== revision.current) return REVALIDATED;
        // 旧服务没有这个接口：不再问，手动「重新读取」照旧能用。
        if (
          error instanceof ApiError &&
          [404, 405, 501].includes(error.status)
        ) {
          headGone.current = true;
          return REVALIDATED;
        }
        return revalidationFailed(message(error));
      }
    },
    [active],
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
    watch,
    next: () =>
      state.page?.next_cursor
        ? load(state.page.next_cursor, state.page)
        : Promise.resolve(),
    mark,
  };
}
