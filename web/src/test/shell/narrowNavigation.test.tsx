import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../../components/AppShell';
import type { View } from '../../types';

const ORDER = ['驾驶舱', '项目', '文档', '协作', '策略'];

function shell(view: View, onNavigate = vi.fn()) {
  return render(
    <AppShell
      view={view}
      onNavigate={onNavigate}
      sourceLabel="测试只读库"
      actionCount={5}
      documentCount={null}
    >
      <p>正文</p>
    </AppShell>,
  );
}

it('主导航只有一份：五格顺序固定，当前项 aria-current=page，窄屏不另渲染一套', async () => {
  const user = userEvent.setup();
  const onNavigate = vi.fn();
  const { rerender } = shell('collab', onNavigate);
  expect(screen.getAllByRole('navigation')).toHaveLength(1);
  const nav = screen.getByRole('navigation', { name: '主导航' });
  const items = within(nav).getAllByRole('button');
  expect(items.map((item) => item.getAttribute('aria-label'))).toEqual(ORDER);
  expect(
    items.filter((item) => item.getAttribute('aria-current') === 'page'),
  ).toEqual([within(nav).getByRole('button', { name: '协作' })]);

  await user.click(within(nav).getByRole('button', { name: '策略' }));
  expect(onNavigate).toHaveBeenCalledWith('policy');
  rerender(
    <AppShell
      view="policy"
      onNavigate={onNavigate}
      sourceLabel="测试只读库"
      actionCount={5}
      documentCount={null}
    >
      <p>正文</p>
    </AppShell>,
  );
  expect(within(nav).getByRole('button', { name: '策略' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  expect(within(nav).getByRole('button', { name: '协作' })).not.toHaveAttribute(
    'aria-current',
  );
});

/**
 * jsdom 不排版，窄屏的「底部五格、44px 触控高度、库名仍在」只能核对样式表本身的约定：
 * 取 shell.css 里 width < 760px 那一段，逐条看声明。
 */
const shellCss = readFileSync(
  join(import.meta.dirname, '../../shell/shell.css'),
  'utf8',
).replace(/\/\*[\s\S]*?\*\//g, '');

function block(source: string, header: string) {
  const start = source.indexOf(header);
  expect(start, `找不到 ${header}`).toBeGreaterThanOrEqual(0);
  const open = source.indexOf('{', start);
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) return source.slice(open + 1, index);
  }
  throw new Error(`${header} 的花括号不成对`);
}

function declarations(source: string, selector: string) {
  const rule = new RegExp(
    `(^|\\})\\s*${selector.replace(/[.[\]()-]/g, '\\$&')}\\s*\\{([^}]*)\\}`,
  ).exec(source);
  expect(rule, `找不到规则 ${selector}`).not.toBeNull();
  return rule![2].replace(/\s+/g, ' ');
}

it('窄屏（<760px）导航固定在底部成五格，每格至少 44px 高', () => {
  const narrow = block(shellCss, '@media (width < 760px)');
  const nav = declarations(narrow, '.shell-nav');
  expect(nav).toContain('position: fixed;');
  expect(nav).toContain('bottom: 0;');
  expect(nav).toContain('grid-template-columns: repeat(5, minmax(0, 1fr));');
  expect(nav).toContain('padding-bottom: env(safe-area-inset-bottom);');
  expect(declarations(narrow, '.shell-nav-item')).toContain(
    'min-height: 44px;',
  );
  // 页面底部留出导航的高度，最后一行内容不被五格盖住
  expect(declarations(narrow, '.shell')).toContain(
    'calc(47px + env(safe-area-inset-bottom) + 24px)',
  );
});

it('当前库名任何宽度都在：窄屏与中等宽度的顶栏网格都给它留了格，样式表里没有藏掉它的规则', () => {
  const narrow = block(shellCss, '@media (width < 760px)');
  expect(declarations(narrow, '.shell-top')).toContain(
    "'brand library status command'",
  );
  // 360px 以下第一行放不下：库名挪到黑线下那一行的左端，仍然在
  const tiny = block(shellCss, '@media (width < 360px)');
  expect(declarations(tiny, '.shell-top')).toContain(
    "'library library tools tools'",
  );
  const medium = block(shellCss, '@media (760px <= width < 960px)');
  expect(declarations(medium, '.shell-top')).toContain(
    "'brand library nav . status command'",
  );
  expect(shellCss).not.toMatch(/\.shell-library\s*\{[^}]*display:\s*none/);
  const library = declarations(shellCss, '.shell-library');
  expect(library).toContain('max-width: 20ch;');
  expect(library).toContain('text-overflow: ellipsis;');
  expect(library).toContain('font-size: var(--type-label);');
});
