import { useCallback, useEffect, useState } from 'react';
import { readHealth } from '../healthApi';
import type { HealthReport } from '../healthApi';

export type HealthStatus =
  | { kind: 'loading' }
  | { kind: 'ready'; report: HealthReport }
  | { kind: 'error'; message: string };

/**
 * 读取本机只读健康页：挂载时读一次，`reload()` 重读。
 * 与工作台快照无关——快照读不出来时照样读；卸载或重读时丢弃过期响应，绝不把旧结果写回。
 */
export function useHealthStatus() {
  const [status, setStatus] = useState<HealthStatus>({ kind: 'loading' });
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    readHealth(controller.signal)
      .then((report) => {
        if (!controller.signal.aborted) setStatus({ kind: 'ready', report });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const message =
          error instanceof Error ? error.message : '健康页读取失败。';
        setStatus({ kind: 'error', message });
      });
    return () => controller.abort();
  }, [revision]);
  // 重读期间显示「正在读取」，不沿用上一次的结论冒充当前状态。
  const reload = useCallback(() => {
    setStatus({ kind: 'loading' });
    setRevision((value) => value + 1);
  }, []);
  return { status, reload };
}
