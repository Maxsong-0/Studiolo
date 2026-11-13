import type { MarkTone } from './markTone';
import standardOnLight from './mark.svg?no-inline';
import standardOnDark from './mark-on-dark.svg?no-inline';
import monoOnLight from './mark-mono.svg?no-inline';
import monoOnDark from './mark-mono-on-dark.svg?no-inline';

/** 标签栏跟系统深浅色走：深色时纸块要换成浅色，否则墨色方块在深色标签栏上看不见。 */
export const DARK_SCHEME_QUERY = '(prefers-color-scheme: dark)';

/** 画法 × 标签栏深浅 → 图标文件地址（构建后是 /assets/ 下带内容哈希的文件）。 */
export function tabIconHref(tone: MarkTone, dark: boolean): string {
  if (tone === 'standard') return dark ? standardOnDark : standardOnLight;
  return dark ? monoOnDark : monoOnLight;
}

export interface TabIcon {
  /** 换画法；和当前一样就不动。 */
  set(tone: MarkTone): void;
  /** 不再跟随系统深浅色。 */
  stop(): void;
}

/**
 * 标签页图标的唯一写入口（设计规范第 14 节）。
 *
 * 页面只声明一个 `<link rel="icon">`：发布检查 scripts/check-preview.py 只认一个图标地址。
 * 所以这里不增不删，只改它的 href：启动时按系统深浅色换上 mono 文件，之后深浅色变了、
 * 外壳报上来的画法变了，都跟着换。找不到那个 link 时什么都不做。
 *
 * scheme 传 `window.matchMedia?.(DARK_SCHEME_QUERY)`；环境给不出（undefined）就一律按浅色。
 */
export function startTabIcon(
  doc: Document,
  scheme: MediaQueryList | undefined,
): TabIcon {
  let tone: MarkTone = 'mono';
  const apply = () => {
    const link = doc.querySelector('link[rel~="icon"]');
    const href = tabIconHref(tone, scheme?.matches ?? false);
    if (link && link.getAttribute('href') !== href)
      link.setAttribute('href', href);
  };
  scheme?.addEventListener('change', apply);
  apply();
  return {
    set(next) {
      if (next === tone) return;
      tone = next;
      apply();
    },
    stop() {
      scheme?.removeEventListener('change', apply);
    },
  };
}
