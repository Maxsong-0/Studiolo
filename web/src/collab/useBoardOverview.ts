import { useCallback, useEffect, useState } from 'react';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import type { BoardOverview } from '../boardOverviewModel';

/** 协作拓扑与「今天的时间线」读的时间窗。 */
export const OVERVIEW_WINDOW_HOURS = 24;
/** 与黑板视图同一时限：超过 10 秒没有结果就放弃这次读取，不一直转圈。 */
const REQUEST_MS = 10_000;

export type OverviewState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'ready'; overview: BoardOverview }
  /** 本机没有装配黑板（汇总接口 404）。 */
  | { kind: 'disabled' }
  | { kind: 'error'; message: string };

type Run = { enabled: boolean; revision: number; state: OverviewState };

const initial = (enabled: boolean): OverviewState =>
  enabled ? { kind: 'loading' } : { kind: 'idle' };

/**
 * 读跨项目黑板汇总（`fetchBoardOverview` 走会话闸门、封闭解码）。只在 enabled 时读；
 * 关掉即取消在途请求，重新打开或 reload 时先回到「读取中」，不先画一帧旧数据。
 * 401、429、503 与解码失败都报错误，只有 404 算「黑板未启用」。
 */
export function useBoardOverview(enabled: boolean): {
  state: OverviewState;
  reload: () => void;
} {
  const [revision, setRevision] = useState(0);
  const [run, setRun] = useState<Run>(() => ({
    enabled,
    revision,
    state: initial(enabled),
  }));
  if (run.enabled !== enabled || run.revision !== revision)
    setRun({ enabled, revision, state: initial(enabled) });

  useEffect(() => {
    if (!enabled) return;
    let settled = false;
    let disposed = false;
    const controller = new AbortController();
    const finish = (state: OverviewState) => {
      if (disposed || settled) return;
      settled = true;
      setRun((current) =>
        current.enabled && current.revision === revision
          ? { ...current, state }
          : current,
      );
    };
    const timer = setTimeout(() => {
      finish({
        kind: 'error',
        message: '黑板汇总读取超时，实时状态不可确认。',
      });
      controller.abort();
    }, REQUEST_MS);
    void (async () => {
      try {
        const overview = await fetchBoardOverview(
          OVERVIEW_WINDOW_HOURS,
          controller.signal,
        );
        finish({ kind: 'ready', overview });
      } catch (error) {
        if (controller.signal.aborted) return;
        finish(
          boardOverviewDisabled(error)
            ? { kind: 'disabled' }
            : {
                kind: 'error',
                message:
                  error instanceof Error ? error.message : '黑板汇总不可用。',
              },
        );
      } finally {
        clearTimeout(timer);
      }
    })();
    return () => {
      disposed = true;
      controller.abort();
      clearTimeout(timer);
    };
  }, [enabled, revision]);

  const reload = useCallback(() => setRevision((value) => value + 1), []);
  const state =
    run.enabled === enabled && run.revision === revision
      ? run.state
      : initial(enabled);
  return { state, reload };
}
