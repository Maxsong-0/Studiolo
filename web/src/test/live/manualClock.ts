import type { AutoRefreshClock } from '../../live/autoRefresh';

/** 让已经就绪的微任务跑完（调度器的一轮是一串 await）。 */
export async function flush() {
  for (let index = 0; index < 20; index += 1) await Promise.resolve();
}

/**
 * 手拨的时钟：定时器按到期时刻排队，`advance` 往前拨并按先后放行。
 * 每放行一个定时器就让微任务跑完，所以「到点 → 读 → 读完再挂下一个定时器」能在一次拨动里走完。
 */
export function manualClock(start = 1_000_000) {
  let now = start;
  let nextId = 1;
  const timers = new Map<number, { at: number; run: () => void }>();
  const clock: AutoRefreshClock = {
    now: () => now,
    setTimer(run, ms) {
      const id = nextId;
      nextId += 1;
      timers.set(id, { at: now + Math.max(0, ms), run });
      return id;
    },
    clearTimer(handle) {
      timers.delete(handle as number);
    },
  };
  return {
    clock,
    now: () => now,
    /** 还挂着几个定时器。 */
    pending: () => timers.size,
    async advance(ms: number) {
      const end = now + ms;
      for (;;) {
        const due = [...timers.entries()]
          .filter(([, timer]) => timer.at <= end)
          .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
        if (!due) break;
        timers.delete(due[0]);
        now = Math.max(now, due[1].at);
        due[1].run();
        await flush();
      }
      now = end;
      await flush();
    },
  };
}

/** 由测试放行的一次应答。 */
export function deferredOutcome<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
