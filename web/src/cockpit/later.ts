/**
 * 「稍后」的记号（T-197，驾驶舱队列）：这一件先放一放，挪到队尾的「稍后再看」里。
 * 只记在这台浏览器的本机存储里，不上服务端，换一台看不到；它还在等你，巨号和角标一个都不少。
 *
 * 规矩：
 * - 一件记三样：它的键的散列、按下时它从哪一刻开始等（`since`）、到期时刻（按下后 {@link LATER_MS}）。
 * - 两种情况作废、回到原位：到期了；或者它有了新进展——现在的 `since` 和记下的不一样。
 *   到期时刻离现在超过 {@link LATER_MS} 的也不认（本机时钟被拨过），免得一件事被放到永远。
 * - 存的是键的散列，不是键本身：工作台待办的键里带着文件路径和行号，路径不落进本机存储。
 *   里面没有任何内容、路径、凭据或搜索词。
 * - 本机存储读不了、写不了（隐私模式、被禁用、配额满）或内容形状不对，一律当作没有记录，不报错；
 *   写不进去时只在这一页里生效。
 * - 最多记 {@link LATER_MAX} 件，超了丢掉最早到期的；每次写都顺手清掉已经到期的。
 */
import { HOUR_MS } from './model';
import { entryKey, type PendingEntry } from './pending';

export interface LaterMark {
  /** 按下「稍后」时，这一件从哪一刻开始等；来源不带时间时为 null。 */
  since: number | null;
  /** 到期时刻（毫秒时间戳）。 */
  until: number;
}

export type LaterMarks = ReadonlyMap<string, LaterMark>;

export const LATER_KEY = 'studiolo.cockpit.later.v1';
/** 放多久。 */
export const LATER_MS = 4 * HOUR_MS;
export const LATER_MAX = 200;

type LaterStorage = Pick<Storage, 'getItem' | 'setItem'>;

/**
 * 键的散列（cyrb53，53 位，写成 36 进制）：只用来认「是不是同一件」，推不回原文。
 * 不是密码学散列，也不需要是：记号只留在本机，撞上的代价不过是某一件被错当成放过。
 */
export function laterId(key: string): string {
  let one = 0xdeadbeef;
  let two = 0x41c6ce57;
  for (let index = 0; index < key.length; index += 1) {
    const code = key.charCodeAt(index);
    one = Math.imul(one ^ code, 2654435761);
    two = Math.imul(two ^ code, 1597334677);
  }
  one =
    Math.imul(one ^ (one >>> 16), 2246822507) ^
    Math.imul(two ^ (two >>> 13), 3266489909);
  two =
    Math.imul(two ^ (two >>> 16), 2246822507) ^
    Math.imul(one ^ (one >>> 13), 3266489909);
  // 第二个数的低 21 位作高位，第一个数的 32 位作低位：合起来 53 位，正好是能精确表示的整数。
  return (4294967296 * (2097151 & two) + (one >>> 0)).toString(36);
}

const ID = /^[0-9a-z]{1,11}$/;

const isMoment = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value > 0;

function parseMarks(raw: string | null): Map<string, LaterMark> {
  const marks = new Map<string, LaterMark>();
  if (!raw) return marks;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return marks;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return marks;
  for (const [id, mark] of Object.entries(value)) {
    if (!ID.test(id) || !mark || typeof mark !== 'object') continue;
    const { since, until } = mark as Record<string, unknown>;
    if (!isMoment(until) || (since !== null && !isMoment(since))) continue;
    marks.set(id, { since, until });
  }
  return marks;
}

/** 还算数的记号：没到期，到期时刻也没有远得离谱。 */
const alive = (mark: LaterMark, now: number) =>
  mark.until > now && mark.until - now <= LATER_MS;

function readMarks(
  storage: LaterStorage | null,
  now: number,
): Map<string, LaterMark> {
  if (!storage) return new Map();
  let parsed: Map<string, LaterMark>;
  try {
    parsed = parseMarks(storage.getItem(LATER_KEY));
  } catch {
    return new Map();
  }
  for (const [id, mark] of parsed) if (!alive(mark, now)) parsed.delete(id);
  return parsed;
}

function writeMarks(storage: LaterStorage | null, marks: LaterMarks): void {
  if (!storage) return;
  try {
    storage.setItem(LATER_KEY, JSON.stringify(Object.fromEntries(marks)));
  } catch {
    // 写不进去就算了：这一页里照样生效，刷新后回到原位。
  }
}

/** 这一件现在是不是在「稍后」里：有记号、没到期、从那以后没有新进展。 */
export function isLater(
  marks: LaterMarks,
  key: string,
  since: number | null,
  now: number,
): boolean {
  if (marks.size === 0) return false;
  const mark = marks.get(laterId(key));
  return mark !== undefined && mark.since === since && alive(mark, now);
}

export interface LaterSplit {
  /** 照常排着的。 */
  active: PendingEntry[];
  /** 放到「稍后再看」里的，各自保持队列里的先后。 */
  later: PendingEntry[];
  /** 「稍后再看」里最早到期的那一刻；没有时为 null。到了这一刻要重排一次。 */
  nextExpiry: number | null;
}

/** 把队列分成两段：照常排着的，和放到「稍后再看」里的。条目本身（含序号）不动。 */
export function splitLater(
  entries: readonly PendingEntry[],
  marks: LaterMarks,
  now: number,
): LaterSplit {
  if (marks.size === 0)
    return { active: [...entries], later: [], nextExpiry: null };
  const active: PendingEntry[] = [];
  const later: PendingEntry[] = [];
  let nextExpiry: number | null = null;
  for (const entry of entries) {
    const mark = marks.get(laterId(entryKey(entry)));
    if (mark && mark.since === entry.since && alive(mark, now)) {
      later.push(entry);
      nextExpiry =
        nextExpiry === null ? mark.until : Math.min(nextExpiry, mark.until);
    } else active.push(entry);
  }
  return { active, later, nextExpiry };
}

export interface LaterStore {
  subscribe(listener: () => void): () => void;
  /** 现在的全部记号；没变时返回同一个对象（useSyncExternalStore 以引用判断是否重画）。 */
  marks(): LaterMarks;
  /** 放到「稍后」。`key` 是队列行的键（pending.ts 的 entryKey），`since` 是它此刻的起点。 */
  later(key: string, since: number | null): void;
  /** 放回队列。 */
  restore(key: string): void;
  /** 别的标签页改了记号：按本机存储里的重读一次。 */
  sync(): void;
  /** 这份记号用的钟：队列排「稍后」时和它对同一个表。 */
  now(): number;
}

export function createLaterStore(environment: {
  storage: LaterStorage | null;
  now: () => number;
}): LaterStore {
  const { storage, now } = environment;
  let marks = readMarks(storage, now());
  const listeners = new Set<() => void>();

  function commit(next: Map<string, LaterMark>, persist: boolean) {
    marks = next;
    if (persist) writeMarks(storage, next);
    listeners.forEach((listener) => listener());
  }

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    marks: () => marks,
    later(key, since) {
      const at = now();
      const id = laterId(key);
      const next = new Map<string, LaterMark>();
      for (const [other, mark] of marks)
        if (other !== id && alive(mark, at)) next.set(other, mark);
      next.set(id, { since, until: at + LATER_MS });
      // 超了上限：丢掉最早到期的。刚记下的这一件到期最晚，轮不到它。
      while (next.size > LATER_MAX) {
        let oldest: string | null = null;
        let oldestUntil = Infinity;
        for (const [other, mark] of next)
          if (mark.until < oldestUntil) {
            oldest = other;
            oldestUntil = mark.until;
          }
        if (oldest === null) break;
        next.delete(oldest);
      }
      commit(next, true);
    },
    restore(key) {
      const id = laterId(key);
      if (!marks.has(id)) return;
      const next = new Map(marks);
      next.delete(id);
      commit(next, true);
    },
    sync() {
      commit(readMarks(storage, now()), false);
    },
    now,
  };
}

function browserStorage(): LaterStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

let shared: LaterStore | null = null;

/** 全站那一份记号：第一次用到时才建，并跟着别的标签页的改动走。 */
export function laterStore(): LaterStore {
  if (shared) return shared;
  const store = createLaterStore({
    storage: browserStorage(),
    now: () => Date.now(),
  });
  window.addEventListener('storage', (event) => {
    if (event.key === LATER_KEY || event.key === null) store.sync();
  });
  shared = store;
  return store;
}
