import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import { COCKPIT_WINDOW_HOURS, type OverviewState } from './model';

/** 黑板汇总的一份读取：读取状态，加上只重读它的动作（拓扑格的「重试」）。 */
export interface OverviewRead {
  state: OverviewState;
  reload: () => void;
}

const message = (error: unknown) =>
  error instanceof Error ? error.message : '黑板汇总读取失败。';

/**
 * 跨项目黑板汇总（`/api/v1/board/overview`）的全站那一份读取（T-162）：由 App 读，
 * 顶栏「驾驶舱 N」角标与驾驶舱（巨号、拓扑、时间线、标题栏）共用同一份，驾驶舱不再另读；
 * 离开驾驶舱时角标照样含黑板里等你的任务。
 *
 * - 有了工作台快照（`ready`）才读：总数要把工作台待办加进去，快照都没有时读了也用不上。
 * - 首次读取在快照到手的那次提交里就发出（layout effect，而不是绘制之后的 effect）：
 *   驾驶舱一出现，它要的请求已经在路上。否则「表格出现之后」再发的请求会落在别处的请求计数
 *   与一次性应答之后，时序随机（会话「一天内切回不发请求」、工作台「刷新失败」两处核对都依赖这一点）。
 * - 一轮刷新（`refreshing` 由真转假，成功或失败）结束就重读：刷新说的是整页读数（沿用驾驶舱原规则）。
 *   还没有快照时的那一轮不算——首次读取跟着快照到手一起发，不会连发两次。
 * - 走与其它内容接口相同的会话闸门（认证请求在途 ≤ 2）。404 记「未启用」，其余失败保留原文。
 * - 重读期间显示读取中，不拿上一次的结论冒充当前状态；卸载或重读时丢弃过期响应，绝不把旧结果写回。
 */
export function useOverviewRead({
  ready,
  refreshing,
}: {
  /** 工作台快照已到手。 */
  ready: boolean;
  /** 工作台快照正在读取或刷新。 */
  refreshing: boolean;
}): OverviewRead {
  const [state, setState] = useState<OverviewState>({ kind: 'loading' });
  const [revision, setRevision] = useState(0);

  useLayoutEffect(() => {
    if (!ready) return;
    const controller = new AbortController();
    fetchBoardOverview(COCKPIT_WINDOW_HOURS, controller.signal)
      .then((overview) => {
        if (!controller.signal.aborted) setState({ kind: 'ready', overview });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setState(
          boardOverviewDisabled(error)
            ? { kind: 'disabled' }
            : { kind: 'error', message: message(error) },
        );
      });
    return () => controller.abort();
  }, [ready, revision]);

  const reload = useCallback(() => {
    setState({ kind: 'loading' });
    setRevision((value) => value + 1);
  }, []);

  // 与首次读取同样在提交的同一拍重读，不拖到绘制之后。
  const wasRefreshing = useRef(false);
  useLayoutEffect(() => {
    if (!ready) return;
    if (wasRefreshing.current && !refreshing) reload();
    wasRefreshing.current = refreshing;
  }, [ready, refreshing, reload]);

  // 同一份读取交给角标与驾驶舱：状态没变就还是同一个对象，驾驶舱不因 App 重绘白算一遍。
  return useMemo(() => ({ state, reload }), [state, reload]);
}
