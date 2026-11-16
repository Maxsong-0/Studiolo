import { useCallback, useMemo, useRef, useState } from 'react';
import { factorRuntime } from '../factorApi';
import { UNKNOWN_AUTH, recentAuthOf, type RecentAuth } from './recentAuth';

/** 本机的单调时钟（毫秒）：只往前走，不受改系统时间影响。 */
export const monotonicNow = () => performance.now();

export interface RecentAuthRead {
  /** 最近一次读到的结论；没读过是 `unknown`。 */
  state: RecentAuth;
  /**
   * 读一次服务端的因子状态并记下结论，同时把结论交回调用方。
   * 认证窗口开着时不要调：会话那一层每读一次就作废上一份在途的读取和已有的快照，窗口里的提交会被打断。
   */
  read: (signal: AbortSignal) => Promise<RecentAuth>;
  /** 倒计时走到头了：把「还有效」改成「没有」。 */
  expire: () => void;
  /** 走字用的时钟（测试里换成自己的）。 */
  now: () => number;
}

/**
 * 近期二次认证还剩多久的那一份读取（T-200）。只读、只为显示：不轮询、不记「已经认证过」，
 * 也不参与要不要弹认证窗口的判断——那由服务端在发布那一刻裁定。
 * 读不到（没有受控会话、接口出错、被更晚的一次读取顶掉）就是「不知道」，界面上什么都不写。
 */
export function useRecentAuth(
  now: () => number = monotonicNow,
): RecentAuthRead {
  const [state, setState] = useState<RecentAuth>(UNKNOWN_AUTH);
  // 只认最后发出的那一次：先发后到的旧结论不盖新的。
  const turn = useRef(0);

  const read = useCallback(
    async (signal: AbortSignal): Promise<RecentAuth> => {
      const mine = (turn.current += 1);
      const runtime = factorRuntime();
      let next: RecentAuth;
      if (!runtime) next = { kind: 'no-session' };
      else {
        try {
          next = recentAuthOf(await runtime.state(signal), now());
        } catch {
          next = UNKNOWN_AUTH;
        }
      }
      if (signal.aborted || mine !== turn.current) return UNKNOWN_AUTH;
      setState(next);
      return next;
    },
    [now],
  );

  const expire = useCallback(() => {
    setState((current) =>
      current.kind === 'valid' ? { kind: 'none' } : current,
    );
  }, []);

  return useMemo(
    () => ({ state, read, expire, now }),
    [state, read, expire, now],
  );
}
