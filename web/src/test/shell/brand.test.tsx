import { render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../../components/AppShell';
import type { MarkTone } from '../../brand/markTone';
import { startTabIcon, tabIconHref } from '../../brand/tabIcon';
import type { TabIcon } from '../../brand/tabIcon';
import { TabIconContext } from '../../brand/tabIconContext';

/** icon 传 null 等于没接标签页图标（上下文的默认值）。 */
function ui(actionCount: number | null, icon: TabIcon | null) {
  return (
    <TabIconContext.Provider value={icon}>
      <AppShell
        view="cockpit"
        onNavigate={vi.fn()}
        sourceLabel="示例库"
        actionCount={actionCount}
        documentCount={12}
      >
        <p>正文</p>
      </AppShell>
    </TabIconContext.Provider>
  );
}

function shell(actionCount: number | null, icon: TabIcon | null = null) {
  return render(ui(actionCount, icon));
}

/** 顶栏里的标志。 */
function topBarMark() {
  const banner = screen.getByRole('banner');
  return within(banner).getByRole('img', { name: 'Studiolo' });
}

/** 页面里那一个图标 link：和 index.html 一样只有一个。用完要 remove。 */
function iconLink() {
  const link = document.createElement('link');
  link.setAttribute('rel', 'icon');
  link.setAttribute('href', '/assets/initial.svg');
  document.head.append(link);
  return link;
}

it('顶栏品牌格是 16px 标志加「工作台」，不再有常亮的红色句点', () => {
  shell(3);
  const brand = screen.getByRole('banner').querySelector('.shell-brand');
  expect(brand?.textContent).toBe('工作台');
  expect(brand).toContainElement(topBarMark());
  expect(topBarMark()).toHaveAttribute('width', '16');
  expect(document.querySelector('.shell-brand-dot')).toBeNull();
});

const tones: [count: number | null, tone: MarkTone][] = [
  [3, 'standard'],
  [0, 'mono'],
  [null, 'mono'],
];

for (const [count, tone] of tones) {
  it(`待我动作是 ${String(count)} 时标志画成 ${tone}，同一个画法报给标签页图标；外壳卸载后回到 mono`, () => {
    const icon: TabIcon = { set: vi.fn(), stop: vi.fn() };
    const view = shell(count, icon);
    expect(topBarMark()).toHaveAttribute('data-tone', tone);
    expect(icon.set).toHaveBeenLastCalledWith(tone);
    view.unmount();
    expect(icon.set).toHaveBeenLastCalledWith('mono');
  });
}

it('待我动作从没有变成有，标志和标签页图标一起变红', () => {
  const icon: TabIcon = { set: vi.fn(), stop: vi.fn() };
  const view = shell(0, icon);
  expect(topBarMark()).toHaveAttribute('data-tone', 'mono');
  expect(icon.set).toHaveBeenLastCalledWith('mono');
  view.rerender(ui(2, icon));
  expect(topBarMark()).toHaveAttribute('data-tone', 'standard');
  expect(icon.set).toHaveBeenLastCalledWith('standard');
});

it('接上真的标签页图标：外壳在时按待我动作换文件，外壳卸载后回到 mono 文件', () => {
  const link = iconLink();
  try {
    const view = shell(2, startTabIcon(document, undefined));
    expect(link.getAttribute('href')).toBe(tabIconHref('standard', false));
    view.unmount();
    expect(link.getAttribute('href')).toBe(tabIconHref('mono', false));
  } finally {
    link.remove();
  }
});

it('没有接标签页图标（单独渲染外壳）时照常渲染，不碰页面的图标 link', () => {
  const link = iconLink();
  try {
    shell(5);
    expect(topBarMark()).toHaveAttribute('data-tone', 'standard');
    expect(link.getAttribute('href')).toBe('/assets/initial.svg');
  } finally {
    link.remove();
  }
});
