import { useEffect, useMemo, useState } from 'react';
import { ApiError, getJson } from './sessionHttp';
import {
  blame,
  blameMark,
  sha256Hex,
  type Blame,
  type BlameMark,
} from './blameModel';

export async function readBlame(path: string, signal: AbortSignal) {
  const query = new URLSearchParams({ path });
  return blame(await getJson(`/api/v1/history/blame?${query}`, signal), path);
}

interface Loaded {
  key: string;
  blame: Blame | null;
  /** 原文快照和服务端当前正文不是同一版：行号对不上，不画。 */
  mismatch: boolean;
  error: string | null;
}

export interface LineBlame extends BlameMark {
  /** 这一行是它那一段的第一行（写缩写、可聚焦）。 */
  first: boolean;
}

/**
 * 阅读器里的逐行归属。默认不读；打开开关才读一次，路径或原文变了重读。
 * 只在两边的正文摘要相同时给出结果：阅读器看的是工作台快照，服务端算的是当前正文。
 */
export function useBlame(
  path: string,
  content: string | null,
  enabled: boolean,
) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [attempt, setAttempt] = useState(0);
  const key = JSON.stringify([path, content?.length ?? null, attempt]);
  useEffect(() => {
    if (!enabled || content === null) return;
    const controller = new AbortController();
    Promise.all([readBlame(path, controller.signal), sha256Hex(content)])
      .then(([value, digest]) => {
        if (controller.signal.aborted) return;
        const mismatch = digest !== value.content_sha256;
        setLoaded({
          key,
          blame: mismatch ? null : value,
          mismatch,
          error: null,
        });
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        const missing =
          cause instanceof ApiError && [404, 405, 501].includes(cause.status);
        setLoaded({
          key,
          blame: null,
          mismatch: false,
          error: missing
            ? '当前服务不提供逐行归属，或这篇文档不在可查范围内。'
            : cause instanceof Error
              ? cause.message
              : '逐行归属读取失败，请重试。',
        });
      });
    return () => controller.abort();
  }, [path, content, enabled, key]);
  const current = enabled && loaded?.key === key ? loaded : null;
  const marks = useMemo(() => {
    if (!current?.blame) return null;
    const lines = new Map<number, LineBlame>();
    for (const run of current.blame.runs) {
      const mark = blameMark(run);
      for (let line = run.from_line; line <= run.to_line; line += 1)
        lines.set(line, { ...mark, first: line === run.from_line });
    }
    return lines;
  }, [current]);
  return {
    marks,
    loading: enabled && content !== null && current === null,
    mismatch: current?.mismatch ?? false,
    error: current?.error ?? null,
    moreHistory: current?.blame?.more_history ?? false,
    retry: () => setAttempt((value) => value + 1),
  };
}
