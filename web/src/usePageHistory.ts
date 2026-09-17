import { useCallback, useEffect, useRef, useState } from 'react';
import { readHistoryFile, readHistoryPage } from './historyApi';
import { ApiError } from './sessionHttp';
import { EDIT_LIMIT } from './canonicalModel';
import type {
  HistoryEntry,
  HistoryFile,
  HistoryPage,
  HistoryScope,
} from './historyModel';

interface HistoryState {
  page: HistoryPage | null;
  selected: HistoryEntry | null;
  file: HistoryFile | null;
  cursor: string | null;
  loading: boolean;
  reading: boolean;
  error: string;
  changed: boolean;
  unavailable: boolean;
}
const initial: HistoryState = {
  page: null,
  selected: null,
  file: null,
  cursor: null,
  loading: true,
  reading: false,
  error: '',
  changed: false,
  unavailable: false,
};
function failure(error: unknown) {
  const code =
    error instanceof ApiError &&
    error.details &&
    typeof error.details === 'object' &&
    'code' in error.details
      ? error.details.code
      : null;
  const changed =
    error instanceof ApiError &&
    error.status === 409 &&
    code === 'history_changed';
  const unavailable =
    error instanceof ApiError &&
    [404, 405, 501].includes(error.status) &&
    code !== 'history_not_found';
  return {
    changed,
    unavailable,
    error: changed
      ? '页面历史已变化，请重新读取第一页。'
      : unavailable
        ? '当前服务未提供可用的页面历史，仍可只读查看原文；不能据此恢复。'
        : error instanceof Error
          ? error.message
          : '历史读取失败，请重试。',
  };
}

/** 页和正文各有取消信号；页变化立即清正文，旧响应不能跨页或作用域回填。 */
export function usePageHistory({ vault_uuid, path }: HistoryScope) {
  const [state, setState] = useState(initial);
  const pageRequest = useRef<AbortController | null>(null);
  const fileRequest = useRef<AbortController | null>(null);
  const readPage = useCallback(
    async (cursor: string | null) => {
      pageRequest.current?.abort();
      fileRequest.current?.abort();
      const request = new AbortController();
      pageRequest.current = request;
      setState({ ...initial, cursor });
      try {
        const page = await readHistoryPage(
          { vault_uuid, path },
          cursor,
          request.signal,
        );
        if (!request.signal.aborted)
          setState({ ...initial, page, cursor, loading: false });
      } catch (error) {
        if (!request.signal.aborted)
          setState({ ...initial, cursor, loading: false, ...failure(error) });
      }
    },
    [vault_uuid, path],
  );
  useEffect(() => {
    void readPage(null);
    return () => {
      pageRequest.current?.abort();
      fileRequest.current?.abort();
    };
  }, [readPage]);
  const select = async (entry: HistoryEntry) => {
    if (
      entry.bytes > EDIT_LIMIT ||
      !state.page?.entries.some(
        (item) => item.id === entry.id && item.revision === entry.revision,
      )
    )
      return;
    fileRequest.current?.abort();
    const request = new AbortController();
    fileRequest.current = request;
    setState((current) => ({
      ...current,
      selected: entry,
      file: null,
      reading: true,
      error: '',
    }));
    try {
      const file = await readHistoryFile(
        { vault_uuid, path },
        entry,
        request.signal,
      );
      if (!request.signal.aborted)
        setState((current) => ({ ...current, file, reading: false }));
    } catch (error) {
      if (!request.signal.aborted) {
        const failed = failure(error);
        setState((current) => ({
          ...current,
          reading: false,
          file: null,
          ...failed,
          selected: null,
          ...(failed.changed ||
          (error instanceof ApiError && [401, 403].includes(error.status))
            ? { page: null }
            : {}),
        }));
      }
    }
  };
  return { ...state, readPage, select };
}
