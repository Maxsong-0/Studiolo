/**
 * 自动刷新的调度（T-196；纯逻辑，不碰 React 与 DOM，时钟由调用方给）。
 *
 * 要跟着刷新的读取各自登记一个「安静重读」函数。到点时按登记的次序一个接一个调，同时在途的最多一个：
 * 服务端认证入口同一时刻只收两个请求（sessionHttp.ts 的闸门），另一个名额留给你点的动作。
 *
 * - 下一轮在「上一次读完」之后一个间隔；重新启用时已经过了这一刻就立刻读，没过就等到那一刻。
 *   所以切走几秒再回来不会多发请求，切走几分钟回来立刻补读。
 * - 一轮里任何一路没读成，这一轮算没成：记下第一处原因，下一轮的间隔加倍，封顶；读成一轮恢复原间隔。
 *   没读成的那一路自己不动数据（留着上一次的），别的几路照常读。
 * - 被叫停的一轮（停用、手动刷新抢先）不算数：在途的那一路作废，后面的不再发，「上一次读完」的时刻不动。
 * - 每一路限时：到时没有结果就作废那一路、记一次没读成，免得一个挂住的请求把后面的轮次全堵死。
 */

/** 一路安静重读的结果。 */
export type Revalidation = { ok: true } | { ok: false; message: string };

/** 读成了；这一轮轮不到它（手动读取在途、那一路没启用）也算这个，不记失败。 */
export const REVALIDATED: Revalidation = { ok: true };

export function revalidationFailed(message: string): Revalidation {
  return { ok: false, message };
}

/** 安静重读：不改成「读取中」；读成了换上新数据，没读成不动数据。信号一旦中止，结果就不许再写回。 */
export type Revalidate = (signal: AbortSignal) => Promise<Revalidation>;

export interface AutoRefreshStatus {
  /** 最近一次读成的时刻（整轮都成，或手动刷新读成），本机时钟毫秒；还没有过是 null。 */
  readAt: number | null;
  /** 最近一轮没读成的第一处原因；读成了是 null。 */
  failure: string | null;
  /** 正在重读。 */
  running: boolean;
  /** 页面上有一会儿没人操作，自动刷新先停着。 */
  idle: boolean;
}

export interface AutoRefreshClock {
  now(): number;
  setTimer(run: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
}

export interface AutoRefreshOptions {
  /** 两轮之间隔多久。 */
  intervalMs?: number;
  /** 连续没读成时间隔加倍，最长到这里。 */
  maxBackoffMs?: number;
  /** 一路最多等多久。 */
  readTimeoutMs?: number;
  /** 到点时有东西挡着（开着弹层）就先不读，隔这么久再看一眼。 */
  blockedRetryMs?: number;
  /** 现在读会不会打扰人：开着弹层时底下换数据会让弹层重挂。 */
  blocked?: () => boolean;
}

export interface AutoRefresh {
  subscribe(listener: () => void): () => void;
  /** 当前状态；没变化时始终是同一个对象。 */
  status(): AutoRefreshStatus;
  /** 登记一路读取；`order` 小的先读。返回注销函数。 */
  register(revalidate: Revalidate, order?: number): () => void;
  /** 该不该读由调用方定（视图、可见、有人在）；`idle` 只用来说明为什么停着。 */
  setEnabled(enabled: boolean, idle?: boolean): void;
  /** 手动读取（首次读取、刷新）读完了：从这一刻重新计时，不紧跟着再读一遍。 */
  noteManualRead(ok: boolean): void;
  /** 立刻读一轮（「现在重读」）；没启用或正在读时不做事。 */
  runNow(): void;
}

/** 提案 F9：页面看得见时 20 秒一读。BUG-035 修好前不调高频率。 */
export const AUTO_REFRESH_INTERVAL_MS = 20_000;
export const AUTO_REFRESH_MAX_BACKOFF_MS = 5 * 60_000;
export const AUTO_REFRESH_READ_TIMEOUT_MS = 10_000;
export const AUTO_REFRESH_BLOCKED_RETRY_MS = 2_000;

const INITIAL: AutoRefreshStatus = {
  readAt: null,
  failure: null,
  running: false,
  idle: false,
};

function describe(cause: unknown): string {
  return cause instanceof Error && cause.message ? cause.message : '读取失败。';
}

export function createAutoRefresh(
  clock: AutoRefreshClock,
  options: AutoRefreshOptions = {},
): AutoRefresh {
  const interval = options.intervalMs ?? AUTO_REFRESH_INTERVAL_MS;
  const maxBackoff = options.maxBackoffMs ?? AUTO_REFRESH_MAX_BACKOFF_MS;
  const readTimeout = options.readTimeoutMs ?? AUTO_REFRESH_READ_TIMEOUT_MS;
  const blockedRetry = options.blockedRetryMs ?? AUTO_REFRESH_BLOCKED_RETRY_MS;
  const listeners = new Set<() => void>();
  const readers: { revalidate: Revalidate; order: number; seq: number }[] = [];
  let registered = 0;
  let status = INITIAL;
  let enabled = false;
  /** 上一次读完的时刻（手动或自动、成或不成）；一次都没读过时是第一次启用的时刻。 */
  let anchor: number | null = null;
  let failures = 0;
  let timer: { handle: unknown } | null = null;
  let round: AbortController | null = null;

  function update(patch: Partial<AutoRefreshStatus>) {
    const next = { ...status, ...patch };
    if (
      next.readAt === status.readAt &&
      next.failure === status.failure &&
      next.running === status.running &&
      next.idle === status.idle
    )
      return;
    status = next;
    for (const listener of [...listeners]) listener();
  }

  function cancelTimer() {
    if (!timer) return;
    clock.clearTimer(timer.handle);
    timer = null;
  }

  function schedule() {
    cancelTimer();
    if (!enabled || round) return;
    const wait = Math.min(interval * 2 ** failures, maxBackoff);
    const due = (anchor ?? clock.now()) + wait;
    timer = { handle: clock.setTimer(tick, Math.max(0, due - clock.now())) };
  }

  function tick() {
    timer = null;
    if (!enabled || round) return;
    if (options.blocked?.()) {
      timer = { handle: clock.setTimer(tick, blockedRetry) };
      return;
    }
    void run();
  }

  /** 读一路：限时；整轮被叫停时立刻了结，结果不计。 */
  function readOne(
    revalidate: Revalidate,
    roundSignal: AbortSignal,
  ): Promise<Revalidation> {
    return new Promise((resolve) => {
      const controller = new AbortController();
      let settled = false;
      const finish = (outcome: Revalidation) => {
        if (settled) return;
        settled = true;
        clock.clearTimer(timeout);
        roundSignal.removeEventListener('abort', stop);
        resolve(outcome);
      };
      const stop = () => {
        controller.abort();
        finish(REVALIDATED);
      };
      const timeout = clock.setTimer(() => {
        controller.abort();
        finish(
          revalidationFailed(
            `超过 ${Math.round(readTimeout / 1000)} 秒没有结果。`,
          ),
        );
      }, readTimeout);
      roundSignal.addEventListener('abort', stop, { once: true });
      let pending: Promise<Revalidation>;
      try {
        pending = revalidate(controller.signal);
      } catch (cause) {
        finish(revalidationFailed(describe(cause)));
        return;
      }
      pending.then(finish, (cause: unknown) =>
        finish(revalidationFailed(describe(cause))),
      );
    });
  }

  async function run() {
    const controller = new AbortController();
    round = controller;
    update({ running: true });
    const queue = [...readers].sort(
      (a, b) => a.order - b.order || a.seq - b.seq,
    );
    let failure: string | null = null;
    for (const reader of queue) {
      if (controller.signal.aborted) break;
      const outcome = await readOne(reader.revalidate, controller.signal);
      if (controller.signal.aborted) break;
      if (!outcome.ok && failure === null) failure = outcome.message;
    }
    round = null;
    if (controller.signal.aborted) {
      update({ running: false });
      schedule();
      return;
    }
    anchor = clock.now();
    if (failure === null) {
      failures = 0;
      update({ running: false, failure: null, readAt: anchor });
    } else {
      failures += 1;
      update({ running: false, failure });
    }
    schedule();
  }

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    status: () => status,
    register(revalidate, order = 0) {
      const entry = { revalidate, order, seq: registered++ };
      readers.push(entry);
      return () => {
        const index = readers.indexOf(entry);
        if (index >= 0) readers.splice(index, 1);
      };
    },
    setEnabled(next, idle = false) {
      update({ idle });
      if (next === enabled) return;
      enabled = next;
      if (!enabled) {
        cancelTimer();
        round?.abort();
        return;
      }
      anchor ??= clock.now();
      schedule();
    },
    noteManualRead(ok) {
      round?.abort();
      anchor = clock.now();
      if (ok) {
        failures = 0;
        update({ readAt: anchor, failure: null });
      }
      schedule();
    },
    runNow() {
      if (!enabled || round) return;
      cancelTimer();
      void run();
    },
  };
}

/** 没有调度器时用的那一个（单独渲染的视图、测试）：什么都不读，也没有「读于」可说。 */
export const NO_AUTO_REFRESH: AutoRefresh = {
  subscribe: () => () => {},
  status: () => INITIAL,
  register: () => () => {},
  setEnabled: () => {},
  noteManualRead: () => {},
  runNow: () => {},
};
