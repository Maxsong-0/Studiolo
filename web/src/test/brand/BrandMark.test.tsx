import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { BrandMark } from '../../brand/BrandMark';

const BRAND = join(import.meta.dirname, '../../brand');

/** 规范里的三个颜色：墨、纸、红。图标文件不走令牌，只能写死这三个。 */
const INK = '#0E0E0E';
const PAPER = '#F1F1EC';
const RED = '#E61919';

/** 四个图标文件：纸块与影子各是什么颜色。 */
const files: [name: string, paper: string, shadow: string][] = [
  ['mark.svg', INK, RED],
  ['mark-on-dark.svg', PAPER, RED],
  ['mark-mono.svg', INK, INK],
  ['mark-mono-on-dark.svg', PAPER, PAPER],
];

it('给了名字是一张有名字的图，画法写在 data-tone 上，边长就是给的像素数', () => {
  render(<BrandMark tone="standard" size={16} label="Studiolo" />);
  const mark = screen.getByRole('img', { name: 'Studiolo' });
  expect(mark).toHaveAttribute('data-tone', 'standard');
  expect(mark).toHaveAttribute('width', '16');
  expect(mark).toHaveAttribute('height', '16');
  expect(mark).toHaveAttribute('viewBox', '0 0 48 48');
});

it('不给名字就只是装饰，读屏跳过', () => {
  const { container } = render(<BrandMark tone="mono" size={24} />);
  expect(screen.queryByRole('img')).toBeNull();
  const mark = container.querySelector('svg.brand-mark');
  expect(mark).toHaveAttribute('aria-hidden', 'true');
  expect(mark).toHaveAttribute('data-tone', 'mono');
});

it('组件和四个图标文件画的是同一个形状，文件只用规范里的三个颜色', () => {
  const { container } = render(<BrandMark tone="standard" size={48} />);
  const paper = container.querySelector('rect');
  const shadow = container.querySelector('path');
  expect(paper).toHaveAttribute('width', '36');
  expect(paper).toHaveAttribute('height', '36');
  const outline = shadow?.getAttribute('d');
  expect(outline).toBe('M42 12H48V48H12V42H42Z');

  for (const [name, paperFill, shadowFill] of files) {
    const source = readFileSync(join(BRAND, name), 'utf8');
    const parsed = new DOMParser().parseFromString(source, 'image/svg+xml');
    const svg = parsed.documentElement;
    expect(svg.getAttribute('viewBox'), name).toBe('0 0 48 48');
    const rect = svg.querySelector('rect');
    expect(rect?.getAttribute('width'), name).toBe('36');
    expect(rect?.getAttribute('height'), name).toBe('36');
    expect(rect?.getAttribute('fill'), name).toBe(paperFill);
    const path = svg.querySelector('path');
    expect(path?.getAttribute('d'), name).toBe(outline);
    expect(path?.getAttribute('fill'), name).toBe(shadowFill);
    // 图标文件作为图片加载，里面的脚本和样式都不该有。
    expect(svg.querySelector('script, style'), name).toBeNull();
  }
});
