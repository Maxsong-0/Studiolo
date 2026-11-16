/**
 * 页面上有没有人在操作（T-196；纯逻辑，时钟由调用方给）。
 *
 * 为什么要有：服务端的会话按「最后一次请求」算闲置期限（crates/auth 的 `last_seen`），每个请求都会把它往后推。
 * 自动刷新要是只看「页面看得见」，一个开着没人管的页面就永远不会闲置登出。所以页面上一段时间没有操作
 * 就停掉自动刷新，让服务端的闲置期限照常走；再有操作立刻恢复。
 * 会话最晚在「最后一次操作 + 这里的时限 + 服务端闲置期限」时到期。
 */
import type { AutoRefreshClock } from './autoRefresh';

/** 多久没有操作算没人在。 */
export const ATTENTION_IDLE_MS = 5 * 60_000;

export interface Attention {
  /** 有操作（指针、按键、滚轮、触摸、切回页面）。高频调用，只记时刻。 */
  touch(): void;
  /** 开始计时；停下后可以再开始。 */
  start(): void;
  stop(): void;
  attended(): boolean;
}

export function createAttention(
  clock: AutoRefreshClock,
  onChange: (attended: boolean) => void,
  idleAfterMs = ATTENTION_IDLE_MS,
): Attention {
  let last = clock.now();
  let attended = true;
  let running = false;
  let timer: { handle: unknown } | null = null;

  function arm() {
    const wait = Math.max(0, last + idleAfterMs - clock.now());
    timer = { handle: clock.setTimer(check, wait) };
  }

  // 一段时限只挂一个定时器：到点时看最后一次操作是什么时候，还没满就按剩下的时间再挂一次。
  function check() {
    timer = null;
    if (clock.now() - last < idleAfterMs) {
      arm();
      return;
    }
    attended = false;
    onChange(false);
  }

  return {
    touch() {
      last = clock.now();
      if (attended) return;
      attended = true;
      if (running) arm();
      onChange(true);
    },
    start() {
      if (running) return;
      running = true;
      if (attended) arm();
    },
    stop() {
      running = false;
      if (!timer) return;
      clock.clearTimer(timer.handle);
      timer = null;
    },
    attended: () => attended,
  };
}
