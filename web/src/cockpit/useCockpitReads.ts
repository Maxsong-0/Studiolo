import { useCallback, useLayoutEffect, useState } from 'react';
import { readHealth } from '../healthApi';
import type { HealthStatus } from '../shell/useHealthStatus';

const message = (error: unknown, fallback: string) =>
  error instanceof Error ? error.message : fallback;

/**
 * 驾驶舱自己的读取：本机健康（`/api/v1/health`），标题栏取自健康的那几格用它。
 * 跨项目黑板汇总不在这里读——它由 App 读一次，与顶栏「驾驶舱 N」角标共用（T-162，见 useOverviewRead）。
 *
 * - 走与其它内容接口相同的会话闸门（认证请求在途 ≤ 2）；失败只影响取自健康的那几格。
 * - 在驾驶舱首次提交的同一拍就发出（layout effect，而不是绘制之后的 effect）：驾驶舱一出现，
 *   它要的请求已经在路上。否则「表格出现之后」再发的请求会落在别处的请求计数与一次性应答之后，
 *   时序随机（会话「一天内切回不发请求」、工作台「刷新失败」两处核对都依赖这一点）。
 * - 与 shell/useHealthStatus 同一套状态：重读期间显示读取中，不拿上一次的结论冒充当前状态；
 *   卸载或重读时丢弃过期响应，绝不把旧结果写回。
 */
export function useCockpitReads() {
  const [health, setHealth] = useState<HealthStatus>({ kind: 'loading' });
  const [healthRevision, setHealthRevision] = useState(0);

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

  /** 重读本机健康（一轮刷新结束后）。 */
  const reloadHealth = useCallback(() => {
    setHealth({ kind: 'loading' });
    setHealthRevision((value) => value + 1);
  }, []);

  return { health, reloadHealth };
}
