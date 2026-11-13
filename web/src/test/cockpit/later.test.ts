import { describe, expect, it, vi } from 'vitest';
import {
  LATER_KEY,
  LATER_MAX,
  LATER_MS,
  createLaterStore,
  isLater,
  laterId,
  splitLater,
} from '../../cockpit/later';
import type { OverviewState } from '../../cockpit/model';
import { entryKey, planPending } from '../../cockpit/pending';
import { fixtureData, policyReady } from './cockpitFixtures';

const T0 = 1_800_000_000_000;

function memory(initial?: unknown) {
  const values = new Map<string, string>();
  if (initial !== undefined)
    values.set(
      LATER_KEY,
      typeof initial === 'string' ? initial : JSON.stringify(initial),
    );
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}

/** 一份可拨钟的记号。 */
function store(storage: ReturnType<typeof memory> | null = memory()) {
  let now = T0;
  const later = createLaterStore({ storage, now: () => now });
  return {
    later,
    storage,
    advance: (ms: number) => {
      now += ms;
    },
    now: () => now,
  };
}

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const { entries } = planPending({
  board: ready,
  policy: policyReady(data.desired),
  inbox: data.workbench.inbox,
  projects: data.workbench.projects,
});

describe('键的散列', () => {
  it('同一个键总得同一个值，不同的键得不同的值；只有小写字母和数字，最长 11 位', () => {
    const ids = entries.map((entry) => laterId(entryKey(entry)));
    expect(new Set(ids).size).toBe(entries.length);
    for (const id of ids) expect(id).toMatch(/^[0-9a-z]{1,11}$/);
    expect(laterId('board:01M31K73HR69MZ4WSNE5ZP1KQR')).toBe(
      laterId('board:01M31K73HR69MZ4WSNE5ZP1KQR'),
    );
    expect(laterId('')).toMatch(/^[0-9a-z]{1,11}$/);
    expect(laterId('inbox:项目/星舟/00-总览/INDEX.md:12:待验收')).not.toBe(
      laterId('inbox:项目/星舟/00-总览/INDEX.md:13:待验收'),
    );
  });
});

describe('「稍后」的记号', () => {
  it('放到稍后：4 小时内算数，到点就不算；放回立刻不算', () => {
    const { later, advance, now } = store();
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(false);
    later.later('board:a', 100);
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(true);
    advance(LATER_MS - 1);
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(true);
    advance(1);
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(false);

    later.later('board:a', 100);
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(true);
    later.restore('board:a');
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(false);
  });

  it('有了新进展（起点变了）就不算；来源不带时间的只看到期', () => {
    const { later, now } = store();
    later.later('board:a', 100);
    expect(isLater(later.marks(), 'board:a', 101, now())).toBe(false);
    expect(isLater(later.marks(), 'board:a', null, now())).toBe(false);
    later.later('inbox:b', null);
    expect(isLater(later.marks(), 'inbox:b', null, now())).toBe(true);
    expect(isLater(later.marks(), 'inbox:b', 5, now())).toBe(false);
    // 别的键不受影响。
    expect(isLater(later.marks(), 'inbox:c', null, now())).toBe(false);
  });

  it('本机存储里只有散列和两个时刻：没有键的原文，没有路径', () => {
    const { later, storage } = store();
    const key = 'inbox:项目/星舟/00-总览/INDEX.md:12:待验收';
    later.later(key, null);
    later.later('board:01M31K73HR69MZ4WSNE5ZP1KQR', 1_700_000_000_000);
    const raw = storage!.values.get(LATER_KEY)!;
    expect(raw).not.toContain('项目');
    expect(raw).not.toContain('INDEX');
    expect(raw).not.toContain('inbox');
    expect(raw).not.toContain('01M31K73HR69MZ4WSNE5ZP1KQR');
    expect(JSON.parse(raw)).toEqual({
      [laterId(key)]: { since: null, until: T0 + LATER_MS },
      [laterId('board:01M31K73HR69MZ4WSNE5ZP1KQR')]: {
        since: 1_700_000_000_000,
        until: T0 + LATER_MS,
      },
    });
    expect([...storage!.values.keys()]).toEqual([LATER_KEY]);
  });

  it('下次打开还在；到期的、时刻远得离谱的、形状不对的都不认', () => {
    const id = laterId('board:a');
    const opened = (value: unknown) =>
      createLaterStore({ storage: memory(value), now: () => T0 }).marks();
    expect([
      ...opened({ [id]: { since: 100, until: T0 + 1000 } }).entries(),
    ]).toEqual([[id, { since: 100, until: T0 + 1000 }]]);
    // 到期了。
    expect(opened({ [id]: { since: 100, until: T0 } }).size).toBe(0);
    // 本机时钟被拨过：到期时刻比 4 小时还远。
    expect(
      opened({ [id]: { since: 100, until: T0 + LATER_MS + 1 } }).size,
    ).toBe(0);
    for (const broken of [
      '不是 JSON',
      '[]',
      'null',
      '"字符串"',
      { [id]: null },
      { [id]: [1, 2] },
      { [id]: { since: 100 } },
      { [id]: { since: '100', until: T0 + 1000 } },
      { [id]: { since: 100, until: '很久以后' } },
      { [id]: { since: 0, until: T0 + 1000 } },
      { [id]: { since: 1.5, until: T0 + 1000 } },
      { 'Not-An-Id': { since: 100, until: T0 + 1000 } },
      { 'board:a': { since: 100, until: T0 + 1000 } },
    ])
      expect(opened(broken).size, JSON.stringify(broken)).toBe(0);
    // 坏的一条不连累好的一条。
    expect(
      opened({
        [id]: { since: null, until: T0 + 1000 },
        坏的: { since: 1, until: 2 },
      }).size,
    ).toBe(1);
  });

  it('每次写都顺手清掉到期的；同一件再放一次从现在重新算', () => {
    const { later, storage, advance, now } = store();
    later.later('board:a', 100);
    advance(LATER_MS - 1000);
    later.later('board:b', 200);
    advance(2000);
    // a 到期了，b 还在；再放一件时 a 被清掉。
    later.later('board:c', 300);
    expect(Object.keys(JSON.parse(storage!.values.get(LATER_KEY)!))).toEqual([
      laterId('board:b'),
      laterId('board:c'),
    ]);
    later.later('board:b', 200);
    expect(later.marks().get(laterId('board:b'))).toEqual({
      since: 200,
      until: now() + LATER_MS,
    });
  });

  it(`最多记 ${LATER_MAX} 件：超了丢掉最早到期的`, () => {
    const { later, advance, now } = store();
    for (let index = 0; index <= LATER_MAX; index += 1) {
      later.later(`board:${index}`, index + 1);
      advance(1);
    }
    expect(later.marks().size).toBe(LATER_MAX);
    expect(isLater(later.marks(), 'board:0', 1, now())).toBe(false);
    expect(isLater(later.marks(), 'board:1', 2, now())).toBe(true);
    expect(
      isLater(later.marks(), `board:${LATER_MAX}`, LATER_MAX + 1, now()),
    ).toBe(true);
  });

  it('变了才通知，没变时给同一个对象；放回一件没放过的不通知', () => {
    const { later } = store();
    const listener = vi.fn();
    const unsubscribe = later.subscribe(listener);
    const before = later.marks();
    expect(later.marks()).toBe(before);
    later.restore('board:a');
    expect(listener).not.toHaveBeenCalled();
    expect(later.marks()).toBe(before);
    later.later('board:a', 100);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(later.marks()).not.toBe(before);
    later.restore('board:a');
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe();
    later.later('board:a', 100);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it('别的标签页改了：按本机存储重读一次', () => {
    const { later, storage, now } = store();
    storage!.values.set(
      LATER_KEY,
      JSON.stringify({
        [laterId('board:a')]: { since: 100, until: T0 + 1000 },
      }),
    );
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(false);
    later.sync();
    expect(isLater(later.marks(), 'board:a', 100, now())).toBe(true);
  });

  it('本机存储用不了：读不了当作没有，写不了只在这一页里生效，都不报错', () => {
    const offline = store(null);
    offline.later.later('board:a', 100);
    expect(isLater(offline.later.marks(), 'board:a', 100, offline.now())).toBe(
      true,
    );
    const broken = createLaterStore({
      storage: {
        getItem: () => {
          throw new Error('被禁用');
        },
        setItem: () => {
          throw new Error('配额满');
        },
      },
      now: () => T0,
    });
    expect(broken.marks().size).toBe(0);
    broken.later('board:a', 100);
    expect(isLater(broken.marks(), 'board:a', 100, T0)).toBe(true);
  });
});

describe('把队列分成两段', () => {
  it('没有记号：全在照常排着的那一段，原样', () => {
    const { later, now } = store();
    const split = splitLater(entries, later.marks(), now());
    expect(split.active).toEqual(entries);
    expect(split.later).toEqual([]);
    expect(split.nextExpiry).toBeNull();
  });

  it('放过的挪到后一段，两段各自保持队列里的先后，序号不变；给出最早到期的时刻', () => {
    const { later, advance, now } = store();
    const [first, second, third] = entries;
    later.later(entryKey(third), third.since);
    advance(1000);
    later.later(entryKey(first), first.since);
    const split = splitLater(entries, later.marks(), now());
    expect(split.later).toEqual([first, third]);
    expect(split.active).toEqual(
      entries.filter((entry) => entry !== first && entry !== third),
    );
    expect(split.active[0]).toBe(second);
    expect(split.later.map((entry) => entry.n)).toEqual([first.n, third.n]);
    expect(split.nextExpiry).toBe(T0 + LATER_MS);
  });

  it('到期或有了新进展的回到原位', () => {
    const { later, advance, now } = store();
    const [first, second] = entries;
    later.later(entryKey(first), first.since);
    later.later(entryKey(second), second.since);
    // 第二件有了新进展：起点变了。
    const moved = entries.map((entry) =>
      entry === second
        ? { ...entry, since: (entry.since ?? 0) + 60_000 }
        : entry,
    );
    const split = splitLater(moved, later.marks(), now());
    expect(split.later).toEqual([first]);
    expect(split.active[0]).toBe(moved[1]);
    advance(LATER_MS);
    expect(splitLater(entries, later.marks(), now()).later).toEqual([]);
  });
});
