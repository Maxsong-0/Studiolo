import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * 对比度守卫（设计规范第 1、11 节；T-163）。jsdom 不加载样式表，只能核对样式表源文本：
 * 从 tokens.css 取两套主题的颜色，按 WCAG 2.1 相对亮度公式算对比度；
 * 再看实心红圈号的规则实际引用了哪两个令牌，两套主题下圈里的字都要 ≥ 4.5:1。
 */

const SRC = join(import.meta.dirname, '../..');
const AA = 4.5;

type Theme = '浅色' | '深色';

/** 读 web/src 下的样式表并去掉注释：注释里提到的令牌与选择器不算数。 */
function stylesheet(path: string): string {
  const source = readFileSync(join(SRC, path), 'utf8');
  return source.replace(/\/\*[\s\S]*?\*\//g, '');
}

/** header 之后第一对花括号里的内容（按花括号配对，可含嵌套）。 */
function block(source: string, header: string): string {
  const start = source.indexOf(header);
  const open = start < 0 ? -1 : source.indexOf('{', start);
  if (open < 0) throw new Error(`找不到 ${header}`);
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, index);
  }
  throw new Error(`${header} 的花括号不成对`);
}

/** 一段声明里的自定义属性：名字 → 值（去首尾空白、转小写）。 */
function customProperties(declarations: string): Map<string, string> {
  const pattern = /(--[\w-]+)\s*:([^;]*);/g;
  const found = new Map<string, string>();
  for (const [, name, value] of declarations.matchAll(pattern))
    found.set(name, value.trim().toLowerCase());
  return found;
}

/** 某套主题下的全部令牌：深色只重定义同名令牌，没重定义的沿用浅色值。 */
function tokens(theme: Theme): Map<string, string> {
  const source = stylesheet('styles/tokens.css');
  const light = customProperties(block(source, ':root'));
  if (theme === '浅色') return light;
  const media = block(source, '@media (prefers-color-scheme: dark)');
  return new Map([...light, ...customProperties(block(media, ':root'))]);
}

/** 令牌的颜色值。只认六位十六进制：改成别的写法时让守卫失败，而不是悄悄漏算。 */
function color(theme: Theme, token: string): string {
  const value = tokens(theme).get(token);
  expect(value, `${theme}主题的 ${token}`).toMatch(/^#[0-9a-f]{6}$/);
  return value!;
}

/** WCAG 2.1 相对亮度：各通道按 sRGB 线性化后加权。 */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((at) => {
    const c = Number.parseInt(hex.slice(at, at + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** 对比度：（较亮 + 0.05）÷（较暗 + 0.05），取值 1–21。 */
function contrast(a: string, b: string): number {
  const [bright, dim] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (bright + 0.05) / (dim + 0.05);
}

/** 选择器恰为 selector 的那条规则的声明，空白压成一个空格。 */
function rule(source: string, selector: string): string {
  const escaped = selector.replace(/[.[\]()-]/g, '\\$&');
  const pattern = new RegExp(`(?:^|[{}])\\s*${escaped}\\s*\\{([^}]*)\\}`);
  const match = pattern.exec(source);
  expect(match, `找不到规则 ${selector}`).not.toBeNull();
  return match![1].replace(/\s+/g, ' ');
}

/** 声明里 property 直接引用的令牌名；不是 var(--…) 的写法就失败。 */
function tokenOf(declarations: string, property: string): string {
  const pattern = new RegExp(
    `(?:^|[;\\s])${property}\\s*:\\s*var\\((--[\\w-]+)\\)\\s*;`,
  );
  const match = pattern.exec(declarations);
  expect(match, `${property} 须直接引用一个令牌`).not.toBeNull();
  return match![1];
}

it('公式自检：黑对白 21:1，同色 1:1，#777777 对白 4.48:1', () => {
  expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 6);
  expect(contrast('#e61919', '#e61919')).toBe(1);
  expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 2);
});

describe.each<Theme>(['浅色', '深色'])('%s主题', (theme) => {
  it.each([
    ['实心红圈号里的数字', '--on-ink', '--signal-text'],
    ['14px 及以下的红字', '--signal-text', '--paper'],
    ['正文', '--ink', '--paper'],
  ])('%s：%s 对 %s ≥ 4.5:1', (_role, text, ground) => {
    const ratio = contrast(color(theme, text), color(theme, ground));
    expect(ratio).toBeGreaterThanOrEqual(AA);
  });

  it('实心红圈号实际引用的字色对圈底 ≥ 4.5:1（圈号组件、拓扑图圈号）', () => {
    const filled = rule(stylesheet('ui/ui.css'), '.ui-circled--filled');
    const collab = stylesheet('styles/collab.css');
    const pairs = [
      [tokenOf(filled, 'color'), tokenOf(filled, 'background')],
      [
        tokenOf(rule(collab, '.topology__marker-n'), 'fill'),
        tokenOf(rule(collab, '.topology__marker-dot'), 'fill'),
      ],
    ];
    for (const [text, ground] of pairs) {
      const ratio = contrast(color(theme, text), color(theme, ground));
      expect(ratio, `${text} 对 ${ground}`).toBeGreaterThanOrEqual(AA);
    }
  });
});

it('圈号组件的实心红圈：圈底与圈框引用 --signal-text，字用 --on-ink', () => {
  const filled = rule(stylesheet('ui/ui.css'), '.ui-circled--filled');
  expect(tokenOf(filled, 'background')).toBe('--signal-text');
  expect(tokenOf(filled, 'border-color')).toBe('--signal-text');
  expect(tokenOf(filled, 'color')).toBe('--on-ink');
  expect(filled).not.toContain('var(--signal)');
});

it('拓扑图的圈号：圆用 --signal-text，数字用 --on-ink', () => {
  const collab = stylesheet('styles/collab.css');
  const dot = rule(collab, '.topology__marker-dot');
  const digit = rule(collab, '.topology__marker-n');
  expect(tokenOf(dot, 'fill')).toBe('--signal-text');
  expect(tokenOf(digit, 'fill')).toBe('--on-ink');
});

it('别处的样式表没有把实心圈号的底改回 --signal', () => {
  const rules = /([^{}]+)\{([^{}]*)\}/g;
  const filledCircle = /\.(?:ui-circled--filled|topology__marker-dot)\b/;
  const overrides: string[] = [];
  for (const path of readdirSync(SRC, { encoding: 'utf8', recursive: true })) {
    if (!path.endsWith('.css')) continue;
    for (const [, selectors, body] of stylesheet(path).matchAll(rules))
      if (filledCircle.test(selectors) && body.includes('var(--signal)'))
        overrides.push(`${path}：${selectors.trim()}`);
  }
  expect(overrides).toEqual([]);
});
