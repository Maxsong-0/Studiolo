import { expect, it, vi } from 'vitest';
import type { MarkTone } from '../../brand/markTone';
import { startTabIcon, tabIconHref } from '../../brand/tabIcon';

/** 一张只声明了一个图标 link 的页面，和 index.html 一样。 */
function page() {
  const doc = document.implementation.createHTMLDocument('');
  const link = doc.createElement('link');
  link.setAttribute('rel', 'icon');
  link.setAttribute('type', 'image/svg+xml');
  link.setAttribute('href', '/assets/initial.svg');
  doc.head.append(link);
  return {
    doc,
    link,
    href: () => link.getAttribute('href'),
    icons: () => doc.querySelectorAll('link[rel~="icon"]').length,
  };
}

/** 可切换的系统深浅色：只实现 startTabIcon 用到的 matches 与 change 事件。 */
function scheme(dark: boolean) {
  const target = new EventTarget();
  let matches = dark;
  Object.defineProperty(target, 'matches', { get: () => matches });
  return {
    list: target as MediaQueryList,
    change(next: boolean) {
      matches = next;
      target.dispatchEvent(new Event('change'));
    },
  };
}

const cases: [MarkTone, boolean][] = [
  ['standard', false],
  ['standard', true],
  ['mono', false],
  ['mono', true],
];

it('四种情况各用一个文件，都是同源的 svg 地址，不是内嵌的 data: 地址（CSP 只放行同源图片）', () => {
  const hrefs = cases.map(([tone, dark]) => tabIconHref(tone, dark));
  expect(new Set(hrefs).size).toBe(4);
  for (const href of hrefs) {
    expect(href.startsWith('data:')).toBe(false);
    expect(href.startsWith('/')).toBe(true);
    expect(new URL(href, 'http://studiolo.test').pathname).toMatch(/\.svg$/);
  }
});

it('启动即按系统深浅色换上 mono 文件；画法和深浅色变了都跟着换，页面始终只有一个图标 link', () => {
  const { doc, href, icons } = page();
  const system = scheme(false);
  const icon = startTabIcon(doc, system.list);
  expect(href()).toBe(tabIconHref('mono', false));

  icon.set('standard');
  expect(href()).toBe(tabIconHref('standard', false));
  system.change(true);
  expect(href()).toBe(tabIconHref('standard', true));
  icon.set('mono');
  expect(href()).toBe(tabIconHref('mono', true));
  system.change(false);
  expect(href()).toBe(tabIconHref('mono', false));
  expect(icons()).toBe(1);
});

it('系统本来就是深色时，启动就用深色标签栏的文件', () => {
  const { doc, href } = page();
  startTabIcon(doc, scheme(true).list);
  expect(href()).toBe(tabIconHref('mono', true));
});

it('画法没变、地址没变时不重写 href', () => {
  const { doc, link } = page();
  const icon = startTabIcon(doc, scheme(false).list);
  const write = vi.spyOn(link, 'setAttribute');
  icon.set('mono');
  expect(write).not.toHaveBeenCalled();
  icon.set('standard');
  icon.set('standard');
  expect(write).toHaveBeenCalledTimes(1);
});

it('stop 之后不再跟随系统深浅色', () => {
  const { doc, href } = page();
  const system = scheme(false);
  const icon = startTabIcon(doc, system.list);
  icon.stop();
  system.change(true);
  expect(href()).toBe(tabIconHref('mono', false));
});

it('环境给不出系统深浅色时一律按浅色', () => {
  const { doc, href } = page();
  const icon = startTabIcon(doc, undefined);
  expect(href()).toBe(tabIconHref('mono', false));
  icon.set('standard');
  expect(href()).toBe(tabIconHref('standard', false));
  expect(() => icon.stop()).not.toThrow();
});

it('页面没有图标 link 时什么都不做，也不自己加一个', () => {
  const doc = document.implementation.createHTMLDocument('');
  const icon = startTabIcon(doc, scheme(true).list);
  expect(() => icon.set('standard')).not.toThrow();
  expect(doc.querySelectorAll('link').length).toBe(0);
});
