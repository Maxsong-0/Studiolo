import { useCallback, useEffect, useRef, useState } from 'react';
import { errorMessage, getWorkbench } from './api';
import {
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
} from './live/autoRefresh';
import type { Workbench } from './types';

/** 两份快照内容是不是一样：代次不同就不必比；代次相同再逐字比（新鲜度可能单独变）。 */
function sameSnapshot(previous: Workbench, next: Workbench): boolean {
  return (
    previous.generation === next.generation &&
    JSON.stringify(previous) === JSON.stringify(next)
  );
}

export function useWorkbench() {
  const [data, setData] = useState<Workbench | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef<AbortController | null>(null);
  const sequence = useRef(0);
  /** 手动读取在途；安静重读看到它就让路。 */
  const manual = useRef(false);

  const load = useCallback(async (refresh = false) => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const current = ++sequence.current;
    manual.current = true;
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
        manual.current = false;
        setLoading(false);
      }
    }
    return null;
  }, []);

  /**
   * 安静重读（自动刷新用，T-196）：读服务端现在这份快照，**不带 `?refresh=true`**——
   * 那个参数会让服务端把整库重新派生一遍（BUG-035 的触发源之一），自动刷新不许碰它。
   *
   * - 不改成「读取中」：页面、⌘K 面板的可用状态都不跟着闪。
   * - 读成了换上新快照，并清掉上一次手动刷新留下的「刷新失败」；内容没变就留着原来那个对象，下面不白白重画。
   * - 没读成不动快照，把原因交回调度器。
   * - 手动读取在途时让路；重读途中有手动读取开始，或信号中止，结果作废。
   */
  const revalidate = useCallback<Revalidate>(async (signal) => {
    if (manual.current) return REVALIDATED;
    const started = sequence.current;
    const stale = () => signal.aborted || started !== sequence.current;
    try {
      const next = await getWorkbench(signal, false);
      if (stale()) return REVALIDATED;
      setData((previous) =>
        previous && sameSnapshot(previous, next) ? previous : next,
      );
      setError(null);
      return REVALIDATED;
    } catch (cause) {
      if (stale()) return REVALIDATED;
      return revalidationFailed(errorMessage(cause));
    }
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
    revalidate,
  };
}
