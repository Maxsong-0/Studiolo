import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import {
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
} from '../live/autoRefresh';
import { COCKPIT_WINDOW_HOURS, type OverviewState } from './model';

/** 黑板汇总的一份读取：读取状态，加上只重读它的动作（「谁在干活」和协作拓扑里的「重试」）。 */
export interface OverviewRead {
  state: OverviewState;
  reload: () => void;
  /** 安静重读（自动刷新用）：不改成读取中，没读成不动数据。 */
  revalidate: Revalidate;
}

/** 与黑板视图同一时限：超过 10 秒没有结果就放弃这次读取，不一直转圈。 */
const REQUEST_MS = 10_000;

const message = (error: unknown) =>
  error instanceof Error ? error.message : '黑板汇总读取失败。';

/**
 * 跨项目黑板汇总（`/api/v1/board/overview`）的全站那一份读取（T-162）：由 App 读，
 * 顶栏「驾驶舱 N」角标、驾驶舱（队列与巨号、谁在干活、自上次查看以来、系统读数）和协作页的拓扑（T-196）
 * 共用同一份，各处的数任何时刻都对得上；离开驾驶舱时角标照样含黑板里等你的任务。
 *
 * - `ready` 才读：有了工作台快照（总数要把工作台待办加进去），或者正在看协作拓扑（快照读不出来时也要能看黑板）。
 * - 首次读取在 `ready` 成立的那次提交里就发出（layout effect，而不是绘制之后的 effect）：
 *   驾驶舱一出现，它要的请求已经在路上。否则「驾驶舱出现之后」再发的请求会落在别处的请求计数
 *   与一次性应答之后，时序随机（会话「一天内切回不发请求」、工作台「刷新失败」两处核对都依赖这一点）。
 * - 一轮刷新（`refreshing` 由真转假，成功或失败）结束就重读：刷新说的是整页读数（沿用驾驶舱原规则）。
 *   首次读取快照的那一轮不算刷新（调用方给 `refreshing` 时就不把它算进去），不会连发两次。
 * - 走与其它内容接口相同的会话闸门（认证请求在途 ≤ 2）。404 记「未启用」，其余失败保留原文；
 *   超过 10 秒没有结果按失败算。
 * - 手动重读（刷新结束、「重试」）期间显示读取中，不拿上一次的结论冒充当前状态；
 *   卸载或重读时丢弃过期响应，绝不把旧结果写回。
 * - 安静重读（`revalidate`，自动刷新每轮调一次）不显示读取中：读成了换上新汇总，没读成留着上一次的、
 *   把原因交回调度器。手动读取在途、还没到该读的时候、本机没装黑板时都让路。
 */
export function useOverviewRead({
  ready,
  refreshing,
}: {
  /** 该读了：工作台快照已到手，或正在看协作拓扑。 */
  ready: boolean;
  /** 已有的工作台快照正在刷新（首次读取不算）。 */
  refreshing: boolean;
}): OverviewRead {
  const [state, setState] = useState<OverviewState>({ kind: 'loading' });
  const [revision, setRevision] = useState(0);
  /** 每发一次手动读取就加一；在途的安静重读看到它变了，就知道自己的结果过期了。 */
  const epoch = useRef(0);
  /** 安静重读要看的现状；提交之后更新（渲染期间不写 ref）。 */
  const latest = useRef({ ready, kind: state.kind });
  useLayoutEffect(() => {
    latest.current = { ready, kind: state.kind };
  });

  useLayoutEffect(() => {
    if (!ready) return;
    epoch.current += 1;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      controller.abort();
      setState({
        kind: 'error',
        message: '黑板汇总读取超时，实时状态不可确认。',
      });
    }, REQUEST_MS);
    fetchBoardOverview(COCKPIT_WINDOW_HOURS, controller.signal)
      .finally(() => window.clearTimeout(timer))
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
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
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

  const revalidate = useCallback<Revalidate>(async (signal) => {
    const { ready: due, kind } = latest.current;
    if (!due || kind === 'loading' || kind === 'disabled') return REVALIDATED;
    const started = epoch.current;
    const stale = () => signal.aborted || started !== epoch.current;
    try {
      const overview = await fetchBoardOverview(COCKPIT_WINDOW_HOURS, signal);
      if (stale()) return REVALIDATED;
      setState({ kind: 'ready', overview });
      return REVALIDATED;
    } catch (error) {
      if (stale()) return REVALIDATED;
      if (boardOverviewDisabled(error)) {
        setState({ kind: 'disabled' });
        return REVALIDATED;
      }
      return revalidationFailed(message(error));
    }
  }, []);

  // 同一份读取交给角标、驾驶舱与协作页：状态没变就还是同一个对象，下面不因 App 重绘白算一遍。
  return useMemo(
    () => ({ state, reload, revalidate }),
    [state, reload, revalidate],
  );
}
