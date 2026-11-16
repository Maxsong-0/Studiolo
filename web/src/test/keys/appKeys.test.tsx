/*
 * 单键快捷键接进整页之后（T-197）：G 再按字母跳视图、「?」开关一览、驾驶舱队列的 J／K／H，
 * 以及不该响应的时候不响应——焦点在输入框里、有弹层开着、单键快捷键关掉了。
 * 「稍后」的记号和单键开关用的是全站那一份（本机存储）：每例跑完清掉再重读，互不相干。
 */
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import { LATER_KEY, laterStore } from '../../cockpit/later';
import { SINGLE_KEYS_KEY, singleKeysPreference } from '../../keys/preference';
import { GO_TARGETS } from '../../keys/shortcuts';
import { parseViewLocation } from '../../viewLocation';
import { overviewResponse, taskId } from '../commands/paletteFixtures';
import {
  COCKPIT_READY,
  defaultApiResponse,
  documents,
  emptyNavigation,
  jsonResponse,
} from '../fixtures';

const STARSHIP = '项目/星舟';

/** 黑板上两条等你验收的；加上工作台里的一条待办，队列一共三行。 */
const board: Parameters<typeof overviewResponse>[0] = [
  {
    id: taskId('A'),
    title: '验收拓扑缩略',
    project: STARSHIP,
    status: 'needs_review',
  },
  {
    id: taskId('B'),
    title: '验收今天的时间线',
    project: STARSHIP,
    status: 'needs_review',
  },
];
const TITLES = ['验收拓扑缩略', '验收今天的时间线', '验收只读工作台'];

const fetchMock = vi.fn<typeof fetch>();

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    if (path.startsWith('/api/v1/board/overview'))
      return jsonResponse(overviewResponse(board));
    if (path.startsWith('/api/document'))
      return jsonResponse({
        path: documents[0].path,
        title: '星舟总览',
        content: '只读文档',
        navigation: emptyNavigation,
      });
    return defaultApiResponse(input);
  });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  // 先卸掉页面，再清全站那两份本机记录：不然清的时候页面还在跟着重画。
  cleanup();
  window.localStorage.clear();
  laterStore().sync();
  singleKeysPreference().sync();
});

const queue = () => screen.getByRole('region', { name: '等你处理' });
const rows = () =>
  Array.from(queue().querySelectorAll<HTMLElement>('.cockpit-queue__row'));
const titles = () =>
  rows().map(
    (row) => row.querySelector('.cockpit-queue__select')!.textContent ?? '',
  );
const selectedIndex = () =>
  rows().findIndex((row) => row.hasAttribute('data-selected'));
const laterList = () => screen.queryByRole('list', { name: '稍后再看' });
const sheet = () => screen.queryByRole('dialog', { name: '键盘快捷键' });
const view = () => parseViewLocation(window.location.hash).view;

/** 打开整页，等驾驶舱的三路都读完（巨号给出 3）。 */
async function openApp() {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', COCKPIT_READY);
  await waitFor(() =>
    expect(
      queue().querySelector('.cockpit-queue__count-value'),
    ).toHaveTextContent('3'),
  );
  expect(titles()).toEqual(TITLES);
  return user;
}

describe('G 再按字母：跳到别的视图', () => {
  it('六个字母各去一个视图，和点顶栏导航一样回到页首', async () => {
    const user = await openApp();
    for (const { key, view: target } of [
      ...GO_TARGETS.slice(1),
      GO_TARGETS[0],
    ]) {
      vi.mocked(window.scrollTo).mockClear();
      await user.keyboard(`g${key}`);
      expect(view(), key).toBe(target);
      expect(window.scrollTo).toHaveBeenCalledWith({
        top: 0,
        left: 0,
        behavior: 'instant',
      });
    }
    // 回到驾驶舱，队列还在。
    await screen.findByRole('heading', COCKPIT_READY);
  });

  it('G 之后按了别的字母：不跳，那个字母也不落到驾驶舱的队列上', async () => {
    const user = await openApp();
    await user.keyboard('gj');
    expect(view()).toBe('cockpit');
    expect(selectedIndex()).toBe(0);
    // 单按 J 照常。
    await user.keyboard('j');
    expect(selectedIndex()).toBe(1);
  });

  it('G H 去驾驶舱，H 不被当成「稍后」；单按 H 才是', async () => {
    const user = await openApp();
    await user.keyboard('gh');
    expect(view()).toBe('cockpit');
    expect(laterList()).toBeNull();
    expect(window.localStorage.getItem(LATER_KEY)).toBeNull();
    await user.keyboard('h');
    expect(laterList()).not.toBeNull();
    expect(titles()).toEqual([TITLES[1], TITLES[2], TITLES[0]]);
    // 巨号和顶栏角标都不变：它还在等你。
    expect(
      queue().querySelector('.cockpit-queue__count-value'),
    ).toHaveTextContent('3');
    // 本机存储里只有散列，没有任务号。
    const raw = window.localStorage.getItem(LATER_KEY)!;
    expect(raw).not.toContain(taskId('A'));
    expect(Object.keys(JSON.parse(raw) as object)).toHaveLength(1);
  });

  it('别的视图上 J／K／H 没有登记：按了没反应，G 照样能跳回来', async () => {
    const user = await openApp();
    await user.keyboard('gp');
    expect(view()).toBe('projects');
    await user.keyboard('jkh');
    expect(view()).toBe('projects');
    expect(window.localStorage.getItem(LATER_KEY)).toBeNull();
    await user.keyboard('gh');
    await screen.findByRole('heading', COCKPIT_READY);
    expect(laterList()).toBeNull();
  });
});

describe('「?」：快捷键一览', () => {
  it('按「?」打开，再按一次关掉；Esc 也关', async () => {
    const user = await openApp();
    expect(sheet()).toBeNull();
    await user.keyboard('?');
    expect(sheet()).toBeInTheDocument();
    await user.keyboard('?');
    expect(sheet()).toBeNull();
    await user.keyboard('{Shift>}?{/Shift}');
    const dialog = sheet()!;
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    expect(sheet()).toBeNull();
  });

  it('一览开着时别的单键都不响应', async () => {
    const user = await openApp();
    await user.keyboard('?');
    await user.keyboard('gp');
    expect(view()).toBe('cockpit');
    await user.keyboard('jh');
    expect(selectedIndex()).toBe(0);
    expect(laterList()).toBeNull();
    // ⌘K 也不在它上面叠开。
    await user.keyboard('{Meta>}k{/Meta}');
    expect(
      screen.queryByRole('dialog', { name: '动作面板' }),
    ).not.toBeInTheDocument();
  });

  it('⌘K 里的「查看键盘快捷键」也能打开它', async () => {
    const user = await openApp();
    await user.keyboard('{Meta>}k{/Meta}');
    const palette = screen.getByRole('dialog', { name: '动作面板' });
    await user.type(
      within(palette).getByRole('combobox', { name: '跳转、搜索或执行动作' }),
      '快捷键',
    );
    expect(
      within(palette).getByRole('option', { name: '查看键盘快捷键' }),
    ).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{Enter}');
    expect(
      screen.queryByRole('dialog', { name: '动作面板' }),
    ).not.toBeInTheDocument();
    expect(sheet()).toBeInTheDocument();
  });
});

describe('不该响应的时候不响应', () => {
  it('焦点在输入框里：字母照常打进去，不跳转、不开一览', async () => {
    const user = await openApp();
    await user.keyboard('gp');
    const search = screen.getByRole('searchbox', { name: '筛选项目' });
    await user.click(search);
    await user.keyboard('gd?jh');
    expect(search).toHaveValue('gd?jh');
    expect(view()).toBe('projects');
    expect(sheet()).toBeNull();
    // 输入框里按过 G 不算开头：离开输入框后单按 D 不跳。
    await user.clear(search);
    await user.type(search, 'g');
    search.blur();
    await user.keyboard('d');
    expect(view()).toBe('projects');
  });

  it('⌘K 面板开着：在它的输入框里打字不触发任何单键', async () => {
    const user = await openApp();
    await user.keyboard('{Meta>}k{/Meta}');
    const palette = screen.getByRole('dialog', { name: '动作面板' });
    const input = within(palette).getByRole('combobox', {
      name: '跳转、搜索或执行动作',
    });
    await user.type(input, 'gp?h');
    expect(input).toHaveValue('gp?h');
    expect(view()).toBe('cockpit');
    expect(sheet()).toBeNull();
    expect(laterList()).toBeNull();
  });

  it('原文阅读器开着：焦点在按钮上也不响应', async () => {
    const user = await openApp();
    await user.keyboard('jj');
    expect(selectedIndex()).toBe(2);
    await user.keyboard('{Enter}');
    const reader = await screen.findByRole('dialog');
    expect(await within(reader).findByText('只读文档')).toBeInTheDocument();
    const close = within(reader).getAllByRole('button')[0];
    close.focus();
    for (const key of ['g', 'p', '?', 'j', 'h'])
      expect(fireEvent.keyDown(close, { key }), key).toBe(true);
    expect(view()).toBe('cockpit');
    expect(sheet()).toBeNull();
    expect(laterList()).toBeNull();
    expect(selectedIndex()).toBe(2);
  });

  it('按着 Ctrl、⌘、Alt，或输入法在组字：不响应', async () => {
    await openApp();
    for (const init of [
      { ctrlKey: true },
      { metaKey: true },
      { altKey: true },
      { isComposing: true },
    ]) {
      for (const key of ['g', 'p', 'j', 'h', '?'])
        fireEvent.keyDown(document.body, { key, ...init });
      expect(view()).toBe('cockpit');
      expect(selectedIndex()).toBe(0);
      expect(laterList()).toBeNull();
      expect(sheet()).toBeNull();
    }
  });
});

describe('单键快捷键的开关', () => {
  it('在一览里关掉：J、G、「?」都不响应，⌘K 照常；从 ⌘K 打开一览再开回来', async () => {
    const user = await openApp();
    await user.keyboard('?');
    await user.click(
      within(sheet()!).getByRole('checkbox', { name: '启用单键快捷键' }),
    );
    expect(window.localStorage.getItem(SINGLE_KEYS_KEY)).toBe('off');
    await user.click(
      within(sheet()!).getByRole('button', { name: '关闭键盘快捷键' }),
    );
    expect(sheet()).toBeNull();

    await user.keyboard('?');
    expect(sheet()).toBeNull();
    await user.keyboard('gp');
    expect(view()).toBe('cockpit');
    (document.activeElement as HTMLElement).blur();
    await user.keyboard('jh');
    expect(selectedIndex()).toBe(0);
    expect(laterList()).toBeNull();

    await user.keyboard('{Meta>}k{/Meta}');
    const palette = screen.getByRole('dialog', { name: '动作面板' });
    await user.type(
      within(palette).getByRole('combobox', { name: '跳转、搜索或执行动作' }),
      '快捷键',
    );
    await user.keyboard('{Enter}');
    const toggle = within(sheet()!).getByRole('checkbox', {
      name: '启用单键快捷键',
    });
    expect(toggle).not.toBeChecked();
    await user.click(toggle);
    expect(window.localStorage.getItem(SINGLE_KEYS_KEY)).toBeNull();
    await user.click(
      within(sheet()!).getByRole('button', { name: '关闭键盘快捷键' }),
    );
    (document.activeElement as HTMLElement).blur();
    await user.keyboard('j');
    expect(selectedIndex()).toBe(1);
  });

  it('上次关掉的：这次打开页面就是关着的', async () => {
    window.localStorage.setItem(SINGLE_KEYS_KEY, 'off');
    singleKeysPreference().sync();
    const user = await openApp();
    await user.keyboard('j?');
    expect(selectedIndex()).toBe(0);
    expect(sheet()).toBeNull();
    expect(within(queue()).getByText('单键快捷键已关闭')).toBeInTheDocument();
  });
});

describe('快照刷新之后', () => {
  it('监听还在：换了一份快照，J 和 G 照常', async () => {
    const user = await openApp();
    await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
    await user.click(screen.getByRole('button', { name: '刷新快照' }));
    await waitFor(() =>
      expect(
        queue().querySelector('.cockpit-queue__count-value'),
      ).toHaveTextContent('3'),
    );
    (document.activeElement as HTMLElement).blur();
    await user.keyboard('j');
    expect(selectedIndex()).toBe(1);
    await user.keyboard('gs');
    expect(view()).toBe('policy');
  });
});
