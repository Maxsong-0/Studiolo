import { useCallback, useLayoutEffect, useState } from 'react';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import { readHealth } from '../healthApi';
import type { HealthStatus } from '../shell/useHealthStatus';
import { COCKPIT_WINDOW_HOURS, type OverviewState } from './model';

const message = (error: unknown, fallback: string) =>
  error instanceof Error ? error.message : fallback;

/**
 * 驾驶舱自己的两路读取：跨项目黑板汇总（`/api/v1/board/overview`）与本机健康（`/api/v1/health`）。
 *
 * - 两路都走与其它内容接口相同的会话闸门（认证请求在途 ≤ 2），各自成败：黑板 404 记「未启用」，
 *   其余失败保留原文；健康失败只影响取自健康的那几格。
 * - 在驾驶舱首次提交的同一拍就发出（layout effect，而不是绘制之后的 effect）：驾驶舱一出现，
 *   它要的请求已经在路上。否则「表格出现之后」再发的请求会落在别处的请求计数与一次性应答之后，
 *   时序随机（会话「一天内切回不发请求」、工作台「刷新失败」两处核对都依赖这一点）。
 * - 与 shell/useHealthStatus 同一套状态：重读期间显示读取中，不拿上一次的结论冒充当前状态；
 *   卸载或重读时丢弃过期响应，绝不把旧结果写回。
 */
export function useCockpitReads(windowHours = COCKPIT_WINDOW_HOURS) {
  const [overview, setOverview] = useState<OverviewState>({ kind: 'loading' });
  const [health, setHealth] = useState<HealthStatus>({ kind: 'loading' });
  const [overviewRevision, setOverviewRevision] = useState(0);
  const [healthRevision, setHealthRevision] = useState(0);

  useLayoutEffect(() => {
    const controller = new AbortController();
    fetchBoardOverview(windowHours, controller.signal)
      .then((data) => {
        if (!controller.signal.aborted)
          setOverview({ kind: 'ready', overview: data });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setOverview(
          boardOverviewDisabled(error)
            ? { kind: 'disabled' }
            : { kind: 'error', message: message(error, '黑板汇总读取失败。') },
        );
      });
    return () => controller.abort();
  }, [windowHours, overviewRevision]);

  useLayoutEffect(() => {
    const controller = new AbortController();
    readHealth(controller.signal)
      .then((report) => {
        if (!controller.signal.aborted) setHealth({ kind: 'ready', report });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setHealth({
          kind: 'error',
          message: message(error, '健康页读取失败。'),
        });
      });
    return () => controller.abort();
  }, [healthRevision]);

  /** 只重读黑板汇总（拓扑格的「重试」）。 */
  const reloadOverview = useCallback(() => {
    setOverview({ kind: 'loading' });
    setOverviewRevision((value) => value + 1);
  }, []);
  /** 两路都重读（一轮刷新结束后）。 */
  const reloadAll = useCallback(() => {
    reloadOverview();
    setHealth({ kind: 'loading' });
    setHealthRevision((value) => value + 1);
  }, [reloadOverview]);

  return { overview, health, reloadOverview, reloadAll };
}
