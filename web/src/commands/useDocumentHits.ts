import { useEffect, useRef, useState } from 'react';
import { ApiError, errorMessage, getSearch } from '../api';
import { queryError } from '../searchQuery';
import type { SearchHit, SearchResponse } from '../types';

/** 至少这么多个字才搜文档：一个字的全文搜索几乎命中全库，只会白占认证名额。 */
export const DOCUMENT_QUERY_MIN = 2;
/** 文档组只列前几条；完整结果去「文档」视图搜。 */
export const DOCUMENT_HIT_LIMIT = 5;
/** 节流：与文档页搜索（useSearch）同一个间隔，停手这么久才发请求。 */
const SEARCH_DELAY_MS = 200;

type Settled =
  | { key: string; phase: 'loading' }
  | { key: string; phase: 'ready'; result: SearchResponse }
  | { key: string; phase: 'error'; error: string; status: number };

export type DocumentHits =
  /** 字数不够，不搜。 */
  | { phase: 'idle' }
  /** 搜索词本身不合规（控制字符、过长），不发请求。 */
  | { phase: 'invalid'; error: string }
  | { phase: 'loading' }
  | {
      phase: 'ready';
      /** 前 {@link DOCUMENT_HIT_LIMIT} 条。 */
      hits: SearchHit[];
      /** 这次返回的命中条数（服务端最多 100 条）。 */
      total: number;
      /** 服务端是否因超过上限截断了结果。 */
      truncated: boolean;
    }
  | { phase: 'error'; error: string; status: number };

/**
 * 动作面板的文档组：沿用文档搜索接口（getSearch，全文模式，带当前快照代次），
 * 输入不少于两个字才搜，停手 200ms 才发；换词、换范围、关面板都会取消在途请求，旧结果绝不串到新输入上。
 */
export function useDocumentHits(
  query: string,
  project: string,
  generation: number,
): DocumentHits & { retry: () => void } {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<Settled | null>(null);
  const sequence = useRef(0);
  const normalized = query.trim();
  const invalid = queryError(query);
  const eligible =
    !invalid && Array.from(normalized).length >= DOCUMENT_QUERY_MIN;
  const key = JSON.stringify([normalized, project, generation, attempt]);

  useEffect(() => {
    if (!eligible) return;
    const current = ++sequence.current;
    const controller = new AbortController();
    const settle = (next: Settled) => {
      if (!controller.signal.aborted && current === sequence.current)
        setState(next);
    };
    const timer = window.setTimeout(() => {
      settle({ key, phase: 'loading' });
      getSearch(
        normalized,
        'full_text',
        project,
        generation,
        controller.signal,
      ).then(
        (result) => settle({ key, phase: 'ready', result }),
        (cause: unknown) =>
          settle({
            key,
            phase: 'error',
            error: errorMessage(cause),
            status: cause instanceof ApiError ? cause.status : 0,
          }),
      );
    }, SEARCH_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [eligible, normalized, project, generation, key]);

  const retry = () => setAttempt((value) => value + 1);
  if (invalid) return { phase: 'invalid', error: invalid, retry };
  if (!eligible) return { phase: 'idle', retry };
  const current = state?.key === key ? state : null;
  if (!current || current.phase === 'loading')
    return { phase: 'loading', retry };
  if (current.phase === 'error')
    return {
      phase: 'error',
      error: current.error,
      status: current.status,
      retry,
    };
  return {
    phase: 'ready',
    hits: current.result.hits.slice(0, DOCUMENT_HIT_LIMIT),
    total: current.result.hits.length,
    truncated: current.result.truncated,
    retry,
  };
}
