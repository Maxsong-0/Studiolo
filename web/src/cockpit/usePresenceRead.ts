import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  REVALIDATED,
  type Revalidate,
  type Revalidation,
} from '../live/autoRefresh';
import { fetchPresence, presenceDisabled } from '../presenceApi';
import type { Presence } from '../presenceModel';
import { presenceAt, presenceDeadline } from './presence';

/** 一次读取最多等这么久：要比自动刷新一轮的时限（10 秒）短，超时才由这里自己说。 */
export const PRESENCE_READ_TIMEOUT_MS = 8_000;

/** 「本机会话」的读取状态；404（本机没装黑板）单列为 disabled。 */
export type PresenceReadState =
  | { kind: 'loading' }
  | { kind: 'ready'; presence: Presence; readAt: number }
  | { kind: 'disabled' }
  /** 读成过，但手里的都到期了、还没有新的：不是读取出错，也不知道现在有没有。 */
  | { kind: 'expired' }
  | { kind: 'error'; message: string };

export interface PresenceRead {
  state: PresenceReadState;
  reload: () => void;
  /** 安静重读（自动刷新用）。 */
  revalidate: Revalidate;
}

const message = (error: unknown) =>
  error instanceof Error ? error.message : '本机会话读取失败。';

/** 读一次；到点不等了（底下的请求也叫停），挂着不回的也有个结果。 */
async function fetchInTime(signal: AbortSignal): Promise<Presence> {
  const own = new AbortController();
  const stop = () => own.abort();
  signal.addEventListener('abort', stop);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error('本机会话读取超时。'));
      own.abort();
    }, PRESENCE_READ_TIMEOUT_MS);
  });
  try {
    return await Promise.race([fetchPresence(own.signal), timeout]);
  } finally {
    clearTimeout(timer);
    signal.removeEventListener('abort', stop);
  }
}

/** 到 `now` 为止还算数的状态：满期的去掉；原来有的全满期、或整份满期没重读成，就换成「过期了」。 */
function expire(state: PresenceReadState, now: number): PresenceReadState {
  if (state.kind !== 'ready') return state;
  const presence = presenceAt(state.presence, state.readAt, now);
  if (presence === state.presence) return state;
  return presence === null ? { kind: 'expired' } : { ...state, presence };
}

/**
 * 「在做什么」状态（`/api/v1/board/presence`，T-237）：只有驾驶舱「谁在干活」用，由驾驶舱自己读。
 *
 * - 和本机健康同一拍发出（layout effect），走同一道会话闸门；失败只影响「本机会话」这一组。
 * - 安静重读没读成就把这一组换成「读不到」，不留上一次的：状态满 5 分钟就过期，留着旧的会把
 *   早就没在跑的说成在跑。别的格子不受影响，所以也不算这一轮没读成。
 * - 没有结果的读取（被中止、一直没重读）也不许留着旧的：每条按自己的岁数在本地到期。
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
    // 睡眠唤醒之后先把早过期的收掉，再等这一次的结果。
    setState((held) => expire(held, Date.now()));
    let next: PresenceReadState;
    try {
      const presence = await fetchInTime(signal);
      next = { kind: 'ready', presence, readAt: Date.now() };
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

  useEffect(() => {
    if (state.kind !== 'ready') return undefined;
    const deadline = presenceDeadline(state.presence, state.readAt);
    const timer = setTimeout(
      // 定时器可能早到一点点：至少按到期那一刻算，否则没收成又不会再来。
      () => setState((held) => expire(held, Math.max(Date.now(), deadline))),
      // 系统钟往回拨也不多等：最多等满一份的有效期。
      Math.min(
        Math.max(0, deadline - Date.now()),
        state.presence.expiresInSecs * 1000,
      ),
    );
    return () => clearTimeout(timer);
  }, [state]);

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
