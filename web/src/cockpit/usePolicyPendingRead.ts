import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
} from '../live/autoRefresh';
import { isAbsent, readDesired } from '../policyApi';
import type { PolicyPendingState } from './model';

/** 策略草案列表的一份读取：读取状态，加上只重读它的动作（队列里的「重试」）。 */
export interface PolicyPendingRead {
  state: PolicyPendingState;
  reload: () => void;
  /** 安静重读（自动刷新用）：不改成读取中，没读成不动数据。 */
  revalidate: Revalidate;
}

const message = (error: unknown) =>
  error instanceof Error ? error.message : '策略草案读取失败。';

/**
 * 策略草案列表（`/api/v1/policy/desired`）的全站那一份读取（T-195）：由 App 读，
 * 顶栏「驾驶舱 N」角标与驾驶舱的「等你」队列共用同一份，驾驶舱不另读。规矩与黑板汇总那一份
 * （useOverviewRead）逐条相同，两路才会同时到、同时重读：
 *
 * - 有了工作台快照（`ready`）才读；首次读取在快照到手的那次提交里就发出（layout effect）。
 * - 一轮刷新（`refreshing` 由真转假，成功或失败）结束就重读；还没有快照时的那一轮不算。
 * - 走会话闸门（认证请求在途 ≤ 2，多出的排队）。404 `policy_plane_absent` 记「未接入」，其余失败保留原文。
 * - 手动重读（刷新结束、「重试」）期间显示读取中，不拿上一次的结论冒充当前状态；卸载或重读时丢弃过期响应。
 * - 安静重读（`revalidate`，自动刷新每轮调一次，T-196）不显示读取中：读成了换上新列表，没读成留着上一次的、
 *   把原因交回调度器。手动读取在途、还没有快照、本入口没装策略平面时都让路。
 *
 * 读到列表时记下本机时刻（`readAt`）：接口不带服务端时刻，草案「已等多久」以它为准。
 * 在策略页发布或撤回之后，这一份要等下一轮刷新才更新（黑板那一路同样如此）。
 */
export function usePolicyPendingRead({
  ready,
  refreshing,
}: {
  /** 工作台快照已到手。 */
  ready: boolean;
  /** 工作台快照正在读取或刷新。 */
  refreshing: boolean;
}): PolicyPendingRead {
  const [state, setState] = useState<PolicyPendingState>({ kind: 'loading' });
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
    readDesired(controller.signal)
      .then((desired) => {
        if (!controller.signal.aborted)
          setState({ kind: 'ready', desired, readAt: Date.now() });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setState(
          isAbsent(error)
            ? { kind: 'absent' }
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

  const revalidate = useCallback<Revalidate>(async (signal) => {
    const { ready: due, kind } = latest.current;
    if (!due || kind === 'loading' || kind === 'absent') return REVALIDATED;
    const started = epoch.current;
    const stale = () => signal.aborted || started !== epoch.current;
    try {
      const desired = await readDesired(signal);
      if (stale()) return REVALIDATED;
      setState({ kind: 'ready', desired, readAt: Date.now() });
      return REVALIDATED;
    } catch (error) {
      if (stale()) return REVALIDATED;
      if (isAbsent(error)) {
        setState({ kind: 'absent' });
        return REVALIDATED;
      }
      return revalidationFailed(message(error));
    }
  }, []);

  // 同一份读取交给角标与驾驶舱：状态没变就还是同一个对象。
  return useMemo(
    () => ({ state, reload, revalidate }),
    [state, reload, revalidate],
  );
}
