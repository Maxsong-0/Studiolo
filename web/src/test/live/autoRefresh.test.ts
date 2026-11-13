/*
 * 自动刷新的调度（T-196，live/autoRefresh.ts）：纯逻辑，用手拨的时钟逐条核对。
 */
import { describe, expect, it, vi } from 'vitest';
import {
  AUTO_REFRESH_INTERVAL_MS,
  AUTO_REFRESH_MAX_BACKOFF_MS,
  createAutoRefresh,
  NO_AUTO_REFRESH,
  REVALIDATED,
  revalidationFailed,
  type Revalidate,
  type Revalidation,
} from '../../live/autoRefresh';
import { deferredOutcome, flush, manualClock } from './manualClock';

const INTERVAL = AUTO_REFRESH_INTERVAL_MS;

/** 一路读取：记下被调了几次、每次的信号；应答由 `answer` 决定（缺省读成）。 */
function reader(
  answer: () => Promise<Revalidation> | Revalidation = () => REVALIDATED,
) {
  const signals: AbortSignal[] = [];
  const revalidate = vi.fn<Revalidate>(async (signal) => {
    signals.push(signal);
    return answer();
  });
  return { revalidate, signals };
}

describe('自动刷新调度', () => {
  it('启用后一个间隔读第一轮，读成记下时刻，再过一个间隔读下一轮', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const one = reader();
    auto.register(one.revalidate);
    auto.setEnabled(true);
    expect(auto.status().readAt).toBeNull();

    await time.advance(INTERVAL - 1);
    expect(one.revalidate).not.toHaveBeenCalled();
    await time.advance(1);
    expect(one.revalidate).toHaveBeenCalledTimes(1);
    expect(auto.status()).toEqual({
      readAt: time.now(),
      failure: null,
      running: false,
      idle: false,
    });

    await time.advance(INTERVAL);
    expect(one.revalidate).toHaveBeenCalledTimes(2);
  });

  it('按登记的次序一个接一个读：前一路没了结，后一路不发', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const first = deferredOutcome<Revalidation>();
    const order: string[] = [];
    // 故意先登记次序靠后的：读的先后只看 order，不看登记先后。
    auto.register(async () => {
      order.push('second');
      return REVALIDATED;
    }, 2);
    auto.register(async () => {
      order.push('first');
      return first.promise;
    }, 1);
    auto.setEnabled(true);

    await time.advance(INTERVAL);
    expect(order).toEqual(['first']);
    expect(auto.status().running).toBe(true);

    first.resolve(REVALIDATED);
    await flush();
    expect(order).toEqual(['first', 'second']);
    expect(auto.status().running).toBe(false);
  });

  it('一路没读成：记第一处原因、读成的时刻不动、后面几路照常读，下一轮间隔加倍直到封顶，读成一轮恢复', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    let healthy = true;
    const flaky = reader(() =>
      healthy ? REVALIDATED : revalidationFailed('认证结果暂不能确认'),
    );
    const alsoFailing = reader(() =>
      healthy ? REVALIDATED : revalidationFailed('第二处原因'),
    );
    const steady = reader();
    auto.register(flaky.revalidate, 0);
    auto.register(alsoFailing.revalidate, 1);
    auto.register(steady.revalidate, 2);
    auto.setEnabled(true);

    await time.advance(INTERVAL);
    const readAt = auto.status().readAt;
    expect(readAt).toBe(time.now());

    healthy = false;
    await time.advance(INTERVAL);
    expect(auto.status()).toMatchObject({
      readAt,
      failure: '认证结果暂不能确认',
    });
    expect(steady.revalidate).toHaveBeenCalledTimes(2);

    // 连着没读成：等 40 秒、80 秒、160 秒、300 秒（封顶）、300 秒。
    let rounds = 2;
    for (const wait of [2, 4, 8, 15, 15].map((factor) => factor * INTERVAL)) {
      expect(wait).toBeLessThanOrEqual(AUTO_REFRESH_MAX_BACKOFF_MS);
      await time.advance(wait - 1);
      expect(flaky.revalidate).toHaveBeenCalledTimes(rounds);
      await time.advance(1);
      rounds += 1;
      expect(flaky.revalidate).toHaveBeenCalledTimes(rounds);
    }

    healthy = true;
    await time.advance(AUTO_REFRESH_MAX_BACKOFF_MS);
    expect(auto.status()).toMatchObject({ readAt: time.now(), failure: null });
    await time.advance(INTERVAL);
    expect(flaky.revalidate).toHaveBeenCalledTimes(rounds + 2);
  });

  it('停用时在途的那一路作废、后面的不再发；这一轮不算数，重新启用时数据过期就立刻读', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const hanging = deferredOutcome<Revalidation>();
    const first = reader(() => hanging.promise);
    const second = reader();
    auto.register(first.revalidate, 0);
    auto.register(second.revalidate, 1);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    expect(first.signals[0].aborted).toBe(false);

    auto.setEnabled(false);
    await flush();
    expect(first.signals[0].aborted).toBe(true);
    expect(second.revalidate).not.toHaveBeenCalled();
    expect(auto.status()).toMatchObject({ running: false, readAt: null });
    expect(time.pending()).toBe(0);

    // 停着的时候不读。
    await time.advance(10 * INTERVAL);
    expect(first.revalidate).toHaveBeenCalledTimes(1);

    hanging.resolve(REVALIDATED);
    auto.setEnabled(true);
    await time.advance(0);
    expect(first.revalidate).toHaveBeenCalledTimes(2);
  });

  it('切走不到一个间隔再回来不多读：等到原定的那一刻', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const one = reader();
    auto.register(one.revalidate);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    expect(one.revalidate).toHaveBeenCalledTimes(1);

    await time.advance(5_000);
    auto.setEnabled(false);
    await time.advance(5_000);
    auto.setEnabled(true);
    await time.advance(INTERVAL - 10_000 - 1);
    expect(one.revalidate).toHaveBeenCalledTimes(1);
    await time.advance(1);
    expect(one.revalidate).toHaveBeenCalledTimes(2);
  });

  it('手动读取读成：记下那一刻、从那一刻重新计时、清掉没读成的记录；在途的一轮作废', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    let outcome: Revalidation = revalidationFailed('读不到');
    const hanging = deferredOutcome<Revalidation>();
    let hang = false;
    const one = reader(() => (hang ? hanging.promise : outcome));
    auto.register(one.revalidate);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    expect(auto.status().failure).toBe('读不到');

    // 加倍后的那一轮在途时，手动刷新抢先读完了。
    hang = true;
    await time.advance(2 * INTERVAL);
    expect(one.revalidate).toHaveBeenCalledTimes(2);
    await time.advance(3_000);
    auto.noteManualRead(true);
    await flush();
    expect(one.signals[1].aborted).toBe(true);
    expect(auto.status()).toEqual({
      readAt: time.now(),
      failure: null,
      running: false,
      idle: false,
    });

    // 从手动读完那一刻起一个间隔（不是加倍后的）。
    hang = false;
    outcome = REVALIDATED;
    await time.advance(INTERVAL - 1);
    expect(one.revalidate).toHaveBeenCalledTimes(2);
    await time.advance(1);
    expect(one.revalidate).toHaveBeenCalledTimes(3);
  });

  it('手动读取没读成：读成的时刻不动，只是重新计时', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const one = reader();
    auto.register(one.revalidate);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    const readAt = auto.status().readAt;

    await time.advance(15_000);
    auto.noteManualRead(false);
    expect(auto.status().readAt).toBe(readAt);
    await time.advance(INTERVAL - 1);
    expect(one.revalidate).toHaveBeenCalledTimes(1);
    await time.advance(1);
    expect(one.revalidate).toHaveBeenCalledTimes(2);
  });

  it('从没读过时也不抢着读：从启用那一刻算一个间隔；启用前手动读完的，从手动读完那一刻算', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const one = reader();
    auto.register(one.revalidate);
    auto.noteManualRead(true);
    await time.advance(8_000);
    auto.setEnabled(true);
    await time.advance(INTERVAL - 8_000 - 1);
    expect(one.revalidate).not.toHaveBeenCalled();
    await time.advance(1);
    expect(one.revalidate).toHaveBeenCalledTimes(1);
  });

  it('到点时有东西挡着就先不读，过一会儿再看；不挡了就读', async () => {
    const time = manualClock();
    let blocked = true;
    const auto = createAutoRefresh(time.clock, {
      blocked: () => blocked,
      blockedRetryMs: 2_000,
    });
    const one = reader();
    auto.register(one.revalidate);
    auto.setEnabled(true);
    await time.advance(INTERVAL + 7_000);
    expect(one.revalidate).not.toHaveBeenCalled();

    blocked = false;
    await time.advance(2_000);
    expect(one.revalidate).toHaveBeenCalledTimes(1);
  });

  it('一路超过时限没有结果：那一路作废、记一次没读成，后面的照常读', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock, { readTimeoutMs: 10_000 });
    const stuck = reader(() => new Promise<Revalidation>(() => {}));
    const after = reader();
    auto.register(stuck.revalidate, 0);
    auto.register(after.revalidate, 1);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    expect(after.revalidate).not.toHaveBeenCalled();

    await time.advance(10_000);
    expect(stuck.signals[0].aborted).toBe(true);
    expect(after.revalidate).toHaveBeenCalledTimes(1);
    expect(auto.status()).toMatchObject({
      failure: '超过 10 秒没有结果。',
      running: false,
    });
  });

  it('读取抛错按没读成算；注销之后不再被调', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const throwing = vi.fn<Revalidate>(() => {
      throw new Error('同步抛出');
    });
    const rejecting = vi.fn<Revalidate>(() =>
      Promise.reject(new Error('异步拒绝')),
    );
    const unregister = auto.register(throwing, 0);
    auto.register(rejecting, 1);
    auto.setEnabled(true);
    await time.advance(INTERVAL);
    expect(auto.status().failure).toBe('同步抛出');
    expect(rejecting).toHaveBeenCalledTimes(1);

    unregister();
    await time.advance(2 * INTERVAL);
    expect(throwing).toHaveBeenCalledTimes(1);
    expect(auto.status().failure).toBe('异步拒绝');
  });

  it('「现在重读」立刻读一轮；正在读或没启用时不做事', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const hanging = deferredOutcome<Revalidation>();
    const one = reader(() => hanging.promise);
    auto.register(one.revalidate);

    auto.runNow();
    await flush();
    expect(one.revalidate).not.toHaveBeenCalled();

    auto.setEnabled(true);
    auto.runNow();
    await flush();
    expect(one.revalidate).toHaveBeenCalledTimes(1);
    auto.runNow();
    await flush();
    expect(one.revalidate).toHaveBeenCalledTimes(1);

    hanging.resolve(REVALIDATED);
    await flush();
    expect(auto.status().readAt).toBe(time.now());
  });

  it('状态没变时是同一个对象，订阅者只在变化时被叫；idle 只是说明为什么停着', async () => {
    const time = manualClock();
    const auto = createAutoRefresh(time.clock);
    const listener = vi.fn();
    const unsubscribe = auto.subscribe(listener);
    const before = auto.status();

    auto.setEnabled(false);
    expect(auto.status()).toBe(before);
    expect(listener).not.toHaveBeenCalled();

    auto.setEnabled(false, true);
    expect(auto.status()).toEqual({ ...before, idle: true });
    expect(listener).toHaveBeenCalledTimes(1);

    auto.setEnabled(true, false);
    expect(auto.status().idle).toBe(false);
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
    auto.noteManualRead(true);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it('没有调度器时用的那一个什么都不做', () => {
    const revalidate = vi.fn<Revalidate>();
    NO_AUTO_REFRESH.register(revalidate)();
    NO_AUTO_REFRESH.setEnabled(true);
    NO_AUTO_REFRESH.noteManualRead(true);
    NO_AUTO_REFRESH.runNow();
    NO_AUTO_REFRESH.subscribe(() => {})();
    expect(revalidate).not.toHaveBeenCalled();
    expect(NO_AUTO_REFRESH.status().readAt).toBeNull();
  });
});
