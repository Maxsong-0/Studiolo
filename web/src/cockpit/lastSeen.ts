/**
 * 「上次查看」的记号（T-195，驾驶舱「自上次查看以来」）：只在这台浏览器里记两样东西——
 * 你上次离开时看到的数据是哪一刻的、那时的提交水位。不进 URL、不上服务端，换浏览器从头算；
 * 里面没有任何内容、路径、凭据或搜索词。
 *
 * 规矩：
 * - 基线在一次「坐下来」期间不变：打开页面时读一次，之后在各视图间来回切、看一眼别的窗口都不重置，
 *   否则离开期间发生了什么，点一下别处就没了。
 * - 记号在离开时才写：页面被切走或关闭的那一刻，把驾驶舱最后一次报上来的读数时刻写进本机存储。
 *   驾驶舱这次没出现过就不写。
 * - 页面一直开着、切走超过 {@link AWAY_MIN_MS} 再切回来，算一次新的「坐下来」：基线换成切走时写下的那一份。
 * - 本机存储读不了、写不了（隐私模式、被禁用、配额满）或内容形状不对，一律当作没有记录，不报错。
 */
import { MINUTE_MS } from './model';

export interface SeenMark {
  /** 你当时看到的数据是哪一刻的（毫秒时间戳）：黑板读到了就取汇总生成时刻，否则取本机时钟。 */
  at: number;
  /** 当时工作台快照的提交水位（十进制串）；快照没给时为 null。 */
  committed: string | null;
}

export const LAST_SEEN_KEY = 'studiolo.cockpit.last-seen.v1';
/** 页面被切走不到这么久不算离开。 */
export const AWAY_MIN_MS = 10 * MINUTE_MS;

type MarkStorage = Pick<Storage, 'getItem' | 'setItem'>;

const WATERMARK = /^(0|[1-9][0-9]{0,19})$/;

function parseMark(raw: string | null): SeenMark | null {
  if (!raw) return null;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const { at, committed } = value as Record<string, unknown>;
  if (typeof at !== 'number' || !Number.isSafeInteger(at) || at <= 0)
    return null;
  if (
    committed !== null &&
    (typeof committed !== 'string' || !WATERMARK.test(committed))
  )
    return null;
  return { at, committed };
}

export function readMark(storage: MarkStorage | null): SeenMark | null {
  if (!storage) return null;
  try {
    return parseMark(storage.getItem(LAST_SEEN_KEY));
  } catch {
    return null;
  }
}

export function writeMark(storage: MarkStorage | null, mark: SeenMark): void {
  if (!storage) return;
  try {
    storage.setItem(
      LAST_SEEN_KEY,
      JSON.stringify({ at: mark.at, committed: mark.committed }),
    );
  } catch {
    // 写不进去就算了：下次打开时没有记录，按「最近 24 小时」显示。
  }
}

export interface SeenTracker {
  subscribe(listener: () => void): () => void;
  /** 这次坐下来期间的基线；没有记录时为 null。基线不变时返回同一个对象。 */
  baseline(): SeenMark | null;
  /** 驾驶舱每拿到一份新读数就报一次：只记在内存里，离开页面时才写进本机存储。 */
  saw(mark: SeenMark): void;
  /** 页面被切走或关闭。 */
  leave(): void;
  /** 页面切回来。 */
  back(): void;
}

export function createSeenTracker(environment: {
  storage: MarkStorage | null;
  now: () => number;
}): SeenTracker {
  const { storage, now } = environment;
  let base = readMark(storage);
  let latest: SeenMark | null = null;
  let written: SeenMark | null = null;
  let leftAt: number | null = null;
  const listeners = new Set<() => void>();
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    baseline: () => base,
    saw(mark) {
      latest = mark;
    },
    leave() {
      if (latest) {
        writeMark(storage, latest);
        written = latest;
      }
      // 「隐藏」与「关闭」可能接连触发：切走的时刻以第一次为准。
      leftAt ??= now();
    },
    back() {
      const away = leftAt === null ? 0 : now() - leftAt;
      leftAt = null;
      if (!written || away < AWAY_MIN_MS || written === base) return;
      base = written;
      listeners.forEach((listener) => listener());
    },
  };
}

function browserStorage(): MarkStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

let shared: SeenTracker | null = null;

/** 全站那一个记号：第一次用到时才建，并接上页面的可见性与关闭事件。 */
export function seenTracker(): SeenTracker {
  if (shared) return shared;
  const tracker = createSeenTracker({
    storage: browserStorage(),
    now: Date.now,
  });
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') tracker.leave();
    else tracker.back();
  });
  window.addEventListener('pagehide', () => tracker.leave());
  shared = tracker;
  return tracker;
}
