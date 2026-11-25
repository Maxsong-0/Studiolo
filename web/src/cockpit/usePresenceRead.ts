import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  REVALIDATED,
  type Revalidate,
  type Revalidation,
} from '../live/autoRefresh';
import { fetchPresence, presenceDisabled } from '../presenceApi';
import type { Presence } from '../presenceModel';

/** 「本机会话」的读取状态；404（本机没装黑板）单列为 disabled。 */
export type PresenceReadState =
  | { kind: 'loading' }
  | { kind: 'ready'; presence: Presence }
  | { kind: 'disabled' }
  | { kind: 'error'; message: string };

export interface PresenceRead {
  state: PresenceReadState;
  reload: () => void;
  /** 安静重读（自动刷新用）。 */
  revalidate: Revalidate;
}

const message = (error: unknown) =>
  error instanceof Error ? error.message : '本机会话读取失败。';

/**
 * 「在做什么」状态（`/api/v1/board/presence`，T-237）：只有驾驶舱「谁在干活」用，由驾驶舱自己读。
 *
 * - 和本机健康同一拍发出（layout effect），走同一道会话闸门；失败只影响「本机会话」这一组。
 * - 安静重读没读成就把这一组换成「读不到」，不留上一次的：状态满 5 分钟就过期，留着旧的会把
 *   早就没在跑的说成在跑。别的格子不受影响，所以也不算这一轮没读成。
 */
export function usePresenceRead(): PresenceRead {
  const [state, setState] = useState<PresenceReadState>({ kind: 'loading' });
  const [revision, setRevision] = useState(0);
  /** 每发一次读取就加一；后到的旧应答看到它变了，就知道自己过期了。 */
  const epoch = useRef(0);
  /** 安静重读要看的现状；提交之后更新（渲染期间不写 ref）。 */
  const kind = useRef(state.kind);
  useLayoutEffect(() => {
    kind.current = state.kind;
  });

  const read = useCallback(async (signal: AbortSignal): Promise<void> => {
    epoch.current += 1;
    const started = epoch.current;
    let next: PresenceReadState;
    try {
      next = { kind: 'ready', presence: await fetchPresence(signal) };
    } catch (error) {
      next = presenceDisabled(error)
        ? { kind: 'disabled' }
        : { kind: 'error', message: message(error) };
    }
    if (!signal.aborted && started === epoch.current) setState(next);
  }, []);

  useLayoutEffect(() => {
    const controller = new AbortController();
    void read(controller.signal);
    return () => controller.abort();
  }, [read, revision]);

  const reload = useCallback(() => {
    setState({ kind: 'loading' });
    setRevision((value) => value + 1);
  }, []);

  const revalidate = useCallback<Revalidate>(
    async (signal): Promise<Revalidation> => {
      if (kind.current === 'loading' || kind.current === 'disabled')
        return REVALIDATED;
      await read(signal);
      return REVALIDATED;
    },
    [read],
  );

  return useMemo(
    () => ({ state, reload, revalidate }),
    [state, reload, revalidate],
  );
}
