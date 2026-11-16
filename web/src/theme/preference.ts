/**
 * 深浅色的手动选择（T-199）：只记在这台浏览器的本机存储里，换一台从头算。
 * 缺省是跟随系统，什么都不存；选了浅色或深色就存一个固定的字（`light`／`dark`），除此之外什么都不存。
 * 选择写在 `<html data-theme>` 上，tokens.css 据此换令牌；没有这个属性就是跟随系统。
 * 本机存储读不了、写不了（隐私模式、被禁用、配额满）时不报错：读不了当作跟随系统，写不了只在这一页里生效。
 *
 * 首屏之前由 `/assets/theme-boot-*.js` 先套一次（web/dev/themeBootPlugin.ts 生成，键、属性名和取值
 * 都从这里拿），免得手动选的和系统相反时先闪一下系统那一套。这个文件因此不能在加载时碰 window：
 * 构建配置也会引入它。
 */
export const THEME_KEY = 'studiolo.theme.v1';
export const THEME_ATTRIBUTE = 'data-theme';
/** 手动能选的两套。「跟随系统」不是一个存下来的值，是没有存。 */
export const MANUAL_THEMES = ['light', 'dark'] as const;
export type ManualTheme = (typeof MANUAL_THEMES)[number];
export type ThemeChoice = 'system' | ManualTheme;

type PreferenceStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface ThemePreference {
  subscribe(listener: () => void): () => void;
  choice(): ThemeChoice;
  set(choice: ThemeChoice): void;
  /** 别的标签页改了选择：按本机存储里的重读一次。 */
  sync(): void;
}

function manual(value: string | null): ManualTheme | null {
  return MANUAL_THEMES.find((theme) => theme === value) ?? null;
}

function read(storage: PreferenceStorage | null): ThemeChoice {
  if (!storage) return 'system';
  try {
    return manual(storage.getItem(THEME_KEY)) ?? 'system';
  } catch {
    return 'system';
  }
}

export function createThemePreference(
  storage: PreferenceStorage | null,
): ThemePreference {
  let choice = read(storage);
  const listeners = new Set<() => void>();
  function change(next: ThemeChoice) {
    if (next === choice) return;
    choice = next;
    listeners.forEach((listener) => listener());
  }
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    choice: () => choice,
    set(next) {
      if (storage)
        try {
          if (next === 'system') storage.removeItem(THEME_KEY);
          else storage.setItem(THEME_KEY, next);
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

/** 把选择套到根元素上：手动选的写属性，跟随系统就把属性拿掉。和当前一样就不动。 */
export function applyTheme(root: Element, choice: ThemeChoice): void {
  if (choice === 'system') {
    if (root.hasAttribute(THEME_ATTRIBUTE))
      root.removeAttribute(THEME_ATTRIBUTE);
  } else if (root.getAttribute(THEME_ATTRIBUTE) !== choice)
    root.setAttribute(THEME_ATTRIBUTE, choice);
}

function browserStorage(): PreferenceStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

let shared: ThemePreference | null = null;

/** 全站那一个选择：第一次用到时才建，并跟着别的标签页的改动走。 */
export function themePreference(): ThemePreference {
  if (shared) return shared;
  const storage = browserStorage();
  const preference = createThemePreference(storage);
  window.addEventListener('storage', (event) => {
    if (
      event.storageArea === storage &&
      (event.key === THEME_KEY || event.key === null)
    )
      preference.sync();
  });
  shared = preference;
  return preference;
}

/**
 * 页面启动时调用一次：把眼下的选择套到 `<html>` 上（启动脚本没跑成时由这里补上），
 * 之后选择一变就跟着换。返回停止跟随的函数。
 */
export function startTheme(
  doc: Document,
  preference: ThemePreference = themePreference(),
): () => void {
  const apply = () => applyTheme(doc.documentElement, preference.choice());
  apply();
  return preference.subscribe(apply);
}
