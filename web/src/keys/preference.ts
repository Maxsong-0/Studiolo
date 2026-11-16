/**
 * 单键快捷键的开关（T-197）：只记在这台浏览器的本机存储里，换一台从头算。
 * 缺省是开；关掉时存一个固定的字 `off`，除此之外什么都不存。
 *
 * 为什么要有开关：单个字符的快捷键会和语音输入、读屏的浏览键撞上，
 * 所以要能关掉（WCAG 2.1.4）。关掉后 ⌘K 不受影响，还能从 ⌘K 里打开一览再开回来。
 * 本机存储读不了、写不了（隐私模式、被禁用、配额满）时不报错：读不了当作开，写不了只在这一页里生效。
 */
export const SINGLE_KEYS_KEY = 'studiolo.keys.single.v1';
const OFF = 'off';

type PreferenceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface SingleKeysPreference {
  subscribe(listener: () => void): () => void;
  enabled(): boolean;
  set(enabled: boolean): void;
  /** 别的标签页改了开关：按本机存储里的重读一次。 */
  sync(): void;
}

function read(storage: PreferenceStorage | null): boolean {
  if (!storage) return true;
  try {
    return storage.getItem(SINGLE_KEYS_KEY) !== OFF;
  } catch {
    return true;
  }
}

export function createSingleKeysPreference(
  storage: PreferenceStorage | null,
): SingleKeysPreference {
  let enabled = read(storage);
  const listeners = new Set<() => void>();
  function change(next: boolean) {
    if (next === enabled) return;
    enabled = next;
    listeners.forEach((listener) => listener());
  }
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    enabled: () => enabled,
    set(next) {
      if (storage)
        try {
          if (next) storage.removeItem(SINGLE_KEYS_KEY);
          else storage.setItem(SINGLE_KEYS_KEY, OFF);
        } catch {
          // 写不进去就只在这一页里生效。
        }
      change(next);
    },
    sync() {
      change(read(storage));
    },
  };
}

function browserStorage(): PreferenceStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

let shared: SingleKeysPreference | null = null;

/** 全站那一个开关：第一次用到时才建，并跟着别的标签页的改动走。 */
export function singleKeysPreference(): SingleKeysPreference {
  if (shared) return shared;
  const preference = createSingleKeysPreference(browserStorage());
  window.addEventListener('storage', (event) => {
    if (event.key === SINGLE_KEYS_KEY || event.key === null) preference.sync();
  });
  shared = preference;
  return preference;
}
