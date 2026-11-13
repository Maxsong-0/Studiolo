import { describe, expect, it, vi } from 'vitest';
import {
  AWAY_MIN_MS,
  LAST_SEEN_KEY,
  createSeenTracker,
  readMark,
  writeMark,
  type SeenMark,
} from '../../cockpit/lastSeen';

/** 内存里的假本机存储：只实现用到的两个方法。 */
function memory(initial?: string) {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(LAST_SEEN_KEY, initial);
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}

/** 读写都抛错的存储：隐私模式、被禁用、配额满。 */
const broken = {
  getItem(): string | null {
    throw new Error('SecurityError');
  },
  setItem(): void {
    throw new Error('QuotaExceededError');
  },
};

const MARK: SeenMark = { at: 1_790_000_000_000, committed: '4821' };
const stored = (mark: unknown) => JSON.stringify(mark);

/** 一个时钟可拨的记号。 */
function tracker(initial?: string) {
  const storage = memory(initial);
  const clock = { now: 1_790_000_100_000 };
  const seen = createSeenTracker({ storage, now: () => clock.now });
  return { storage, clock, seen };
}

describe('上次查看的记号：读写本机存储', () => {
  it('写下去的只有时刻和提交水位两样，读回来原样', () => {
    const storage = memory();
    writeMark(storage, MARK);
    expect([...storage.values.keys()]).toEqual([LAST_SEEN_KEY]);
    expect(JSON.parse(storage.values.get(LAST_SEEN_KEY)!)).toEqual({
      at: MARK.at,
      committed: '4821',
    });
    expect(readMark(storage)).toEqual(MARK);
    // 当时快照没给水位：照实记成 null。
    writeMark(storage, { at: MARK.at, committed: null });
    expect(readMark(storage)).toEqual({ at: MARK.at, committed: null });
    // 多带的字段不写进去。
    writeMark(storage, { ...MARK, title: '不该出现' } as SeenMark);
    expect(storage.values.get(LAST_SEEN_KEY)).not.toContain('不该出现');
  });

  it.each<[string, string]>([
    ['空串', ''],
    ['不是 JSON', '{at:1'],
    ['null', 'null'],
    ['数组', '[1790000000000,"1"]'],
    ['时刻不是数', stored({ at: '1790000000000', committed: '1' })],
    ['时刻是 0', stored({ at: 0, committed: '1' })],
    ['时刻是负数', stored({ at: -5, committed: '1' })],
    ['时刻带小数', stored({ at: 1.5, committed: '1' })],
    ['时刻超出安全整数', '{"at":1e400,"committed":"1"}'],
    ['没有水位字段', stored({ at: 1_790_000_000_000 })],
    ['水位是数字', stored({ at: 1_790_000_000_000, committed: 4821 })],
    ['水位带前导零', stored({ at: 1_790_000_000_000, committed: '04821' })],
    ['水位是负数', stored({ at: 1_790_000_000_000, committed: '-1' })],
    ['水位不是十进制', stored({ at: 1_790_000_000_000, committed: '0x1f' })],
    [
      '水位超过 20 位',
      stored({ at: 1_790_000_000_000, committed: '1'.repeat(21) }),
    ],
  ])('存储里的内容形状不对（%s）：当作没有记录', (_, raw) => {
    expect(readMark(memory(raw))).toBeNull();
  });

  it('存储读不了、写不了或根本没有：当作没有记录，不报错', () => {
    expect(readMark(broken)).toBeNull();
    expect(() => writeMark(broken, MARK)).not.toThrow();
    expect(readMark(null)).toBeNull();
    expect(() => writeMark(null, MARK)).not.toThrow();
    const seen = createSeenTracker({ storage: broken, now: () => 0 });
    expect(seen.baseline()).toBeNull();
    seen.saw(MARK);
    expect(() => seen.leave()).not.toThrow();
  });
});

describe('上次查看的记号：一次「坐下来」', () => {
  it('基线在打开时读一次；看到新读数只记在内存里，离开时才写', () => {
    const { storage, seen } = tracker(stored(MARK));
    expect(seen.baseline()).toEqual(MARK);
    // 同一个对象：订阅它的组件不会因为每次取到新对象而反复重画。
    expect(seen.baseline()).toBe(seen.baseline());
    const next: SeenMark = { at: MARK.at + 60_000, committed: '4830' };
    seen.saw(next);
    expect(readMark(storage)).toEqual(MARK);
    expect(seen.baseline()).toEqual(MARK);
    seen.leave();
    expect(readMark(storage)).toEqual(next);
    // 离开的那一刻基线不变：切回来不够久的话，看到的还是离开前那一份。
    expect(seen.baseline()).toEqual(MARK);
  });

  it('驾驶舱这次没出现过就不写', () => {
    const { storage, seen } = tracker(stored(MARK));
    seen.leave();
    expect(readMark(storage)).toEqual(MARK);
    const empty = tracker();
    empty.seen.leave();
    expect(empty.storage.values.size).toBe(0);
  });

  it('切走不到十分钟再回来不算离开：基线不动，也不通知', () => {
    const { clock, seen } = tracker(stored(MARK));
    const listener = vi.fn();
    seen.subscribe(listener);
    seen.saw({ at: MARK.at + 60_000, committed: '4830' });
    seen.leave();
    clock.now += AWAY_MIN_MS - 1;
    seen.back();
    expect(seen.baseline()).toEqual(MARK);
    expect(listener).not.toHaveBeenCalled();
  });

  it('切走满十分钟再回来算一次新的「坐下来」：基线换成切走时写下的那一份，并通知订阅者', () => {
    const { clock, seen } = tracker(stored(MARK));
    const listener = vi.fn();
    const unsubscribe = seen.subscribe(listener);
    const dropped = vi.fn();
    seen.subscribe(dropped)();
    const next: SeenMark = { at: MARK.at + 60_000, committed: '4830' };
    seen.saw(next);
    seen.leave();
    clock.now += AWAY_MIN_MS;
    seen.back();
    expect(seen.baseline()).toEqual(next);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(dropped).not.toHaveBeenCalled();

    // 回来后没看到新读数又离开一次：写下的还是同一份，基线没变就不再通知。
    seen.leave();
    clock.now += AWAY_MIN_MS;
    seen.back();
    expect(seen.baseline()).toEqual(next);
    expect(listener).toHaveBeenCalledTimes(1);

    // 退订之后不再被通知。
    unsubscribe();
    seen.saw({ at: MARK.at + 120_000, committed: '4831' });
    seen.leave();
    clock.now += AWAY_MIN_MS;
    seen.back();
    expect(seen.baseline()).toEqual({
      at: MARK.at + 120_000,
      committed: '4831',
    });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('「隐藏」和「关闭」接连触发：切走的时刻以第一次为准，写下的是最新读数', () => {
    const { storage, clock, seen } = tracker();
    seen.saw(MARK);
    seen.leave();
    clock.now += AWAY_MIN_MS - 1_000;
    const newer: SeenMark = { at: MARK.at + 1, committed: '4822' };
    seen.saw(newer);
    seen.leave();
    expect(readMark(storage)).toEqual(newer);
    clock.now += 1_000;
    // 从第一次切走算起已满十分钟。
    seen.back();
    expect(seen.baseline()).toEqual(newer);
  });

  it('没切走过就「回来」：什么都不变', () => {
    const { seen } = tracker(stored(MARK));
    const listener = vi.fn();
    seen.subscribe(listener);
    seen.saw({ at: MARK.at + 60_000, committed: '4830' });
    seen.back();
    expect(seen.baseline()).toEqual(MARK);
    expect(listener).not.toHaveBeenCalled();
  });

  it('第一次用（没有记录）：这次坐下来基线一直是空的，离开后写下，下次打开才有', () => {
    const { storage, clock, seen } = tracker();
    expect(seen.baseline()).toBeNull();
    seen.saw(MARK);
    expect(seen.baseline()).toBeNull();
    seen.leave();
    clock.now += 60_000;
    seen.back();
    expect(seen.baseline()).toBeNull();
    // 下次打开页面（新的记号）读到的就是它。
    const reopened = createSeenTracker({ storage, now: () => clock.now });
    expect(reopened.baseline()).toEqual(MARK);
  });
});
