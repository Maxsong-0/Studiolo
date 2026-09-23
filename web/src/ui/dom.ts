import type { CSSProperties } from 'react';

/** 拼接类名，跳过空值。 */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(' ');
}

/**
 * 只用来给 ui.css 传数据驱动的 CSS 自定义属性（列数、跨度等）。
 * React 经 CSSOM 的 setProperty 写入，不受 CSP style-src 限制；样式规则本身仍全部在 ui.css 里。
 */
export function cssVars(
  vars: Record<`--${string}`, string | undefined>,
): CSSProperties {
  const style: Record<string, string> = {};
  for (const [name, value] of Object.entries(vars)) {
    if (value !== undefined) style[name] = value;
  }
  return style as CSSProperties;
}
