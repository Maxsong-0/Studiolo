import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type Mock,
} from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import { QueueCell } from '../../cockpit/QueueCell';
import { DETAIL_LIMIT } from '../../cockpit/detail';
import { LATER_KEY, LATER_MS, createLaterStore } from '../../cockpit/later';
import type { OverviewState, PolicyPendingState } from '../../cockpit/model';
import { planPending } from '../../cockpit/pending';
import { overviewLabels } from '../../collab/executorGrouping';
import { createSingleKeysPreference } from '../../keys/preference';
import type { Action, OpenSource } from '../../types';
import { KeysHarness } from '../keys/KeysHarness';
import { NOW, fixtureData, policyReady } from './cockpitFixtures';

/** 假数据那份 AI 提案在队列里的标题。 */
const PROPOSAL = '能力集 claude';

function memory() {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

const data = fixtureData();
const READY: OverviewState = { kind: 'ready', overview: data.overview };
const DRAFTS = policyReady(data.desired);
const titleOf = (id: string) =>
  data.overview.items.find((item) => item.id === id)!.title;

interface Sources {
  board?: OverviewState;
  policy?: PolicyPendingState;
  inbox?: readonly Action[];
}

/** 队列往外交的三个回调，各自带着组件要的签名。 */
interface Handlers {
  onOpen: Mock<OpenSource>;
  onOpenTask: Mock<(taskId: string) => void>;
  onOpenPolicy: Mock<() => void>;
}

function cell(
  sources: Sources,
  environment: {
    later: ReturnType<typeof createLaterStore>;
    handlers: Handlers;
  },
) {
  const board = sources.board ?? READY;
  const policy = sources.policy ?? DRAFTS;
  const plan = planPending({
    board,
    policy,
    inbox: sources.inbox ?? data.workbench.inbox,
    projects: data.workbench.projects,
  });
  return (
    <QueueCell
      plan={plan}
      board={board}
      policy={policy}
      labels={
        board.kind === 'ready' ? overviewLabels(board.overview) : undefined
      }
      asOf="20:31"
      onRetryPolicy={() => {}}
      later={environment.later}
      {...environment.handlers}
    />
  );
}

/** 渲染队列这一格，外面包着全站按键监听；`live: false` 时不包（单独渲染的视图）。 */
function renderQueue(sources: Sources = {}, { live = true } = {}) {
  const storage = memory();
  const later = createLaterStore({ storage, now: () => Date.now() });
  const preference = createSingleKeysPreference(memory());
  const handlers: Handlers = {
    onOpen: vi.fn<OpenSource>(),
    onOpenTask: vi.fn<(taskId: string) => void>(),
    onOpenPolicy: vi.fn<() => void>(),
  };
  const onGo = vi.fn();
  const environment = { later, handlers };
  const wrap = (next: Sources) =>
    live ? (
      <KeysHarness preference={preference} onGo={onGo}>
        {cell(next, environment)}
      </KeysHarness>
    ) : (
      cell(next, environment)
    );
  const view = render(wrap(sources));
  return {
    storage,
    later,
    preference,
    handlers,
    onGo,
    rerender: (next: Sources) => view.rerender(wrap(next)),
  };
}

const queue = () => screen.getByRole('region', { name: '等你处理' });
const rows = () =>
  Array.from(queue().querySelectorAll<HTMLElement>('.cockpit-queue__row'));
const titles = () =>
  rows().map(
    (row) => row.querySelector('.cockpit-queue__select')!.textContent ?? '',
  );
const numbers = () =>
  rows().map((row) => row.querySelector('.cockpit-queue__n')!.textContent);
const selectedIndex = () =>
  rows().findIndex((row) => row.hasAttribute('data-selected'));
const primary = () =>
  queue().querySelector<HTMLElement>('[data-queue-primary]');
const laterList = () =>
  within(queue()).queryByRole('list', { name: '稍后再看' });
const laterTitles = () =>
  Array.from(
    laterList()?.querySelectorAll('.cockpit-queue__select') ?? [],
    (node) => node.textContent ?? '',
  );
const countValue = () =>
  queue().querySelector<HTMLElement>('.cockpit-queue__count-value')!;
const press = (key: string, init: KeyboardEventInit = {}) =>
  fireEvent.keyDown(document.activeElement ?? document.body, { key, ...init });

/** 假数据的队列：提案、三条黑板任务（按最后一次进展的先后）、工作台待办。 */
const ORDER = [
  PROPOSAL,
  titleOf(TASK.T093),
  titleOf(TASK.T106),
  titleOf(TASK.T108),
];
const TOTAL = ORDER.length + data.workbench.inbox.length;

beforeEach(() => {
  // 假数据的「此刻」是 2026-09-22 20:31：本机时钟拨到同一刻。
  vi.spyOn(Date, 'now').mockReturnValue(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('队列的键盘：J／K 上下选', () => {
  it('J 选下一行、K 选上一行，焦点跟着落到那一行的主按钮；到头不绕回', () => {
    renderQueue();
    expect(titles().slice(0, ORDER.length)).toEqual(ORDER);
    expect(selectedIndex()).toBe(0);
    expect(document.body).toHaveFocus();

    press('j');
    expect(selectedIndex()).toBe(1);
    expect(primary()).toHaveFocus();
    expect(primary()).toHaveAccessibleName(
      `${titleOf(TASK.T093)}，在协作页查看`,
    );
    expect(rows()[1].contains(primary())).toBe(true);
    // 选中行滚进视野（jsdom 里只核对调了没有）。
    expect(rows()[1].scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' });

    press('K');
    expect(selectedIndex()).toBe(0);
    expect(primary()).toHaveFocus();
    press('k');
    expect(selectedIndex()).toBe(0);
    expect(primary()).toHaveFocus();

    for (let step = 0; step < TOTAL + 2; step += 1) press('j');
    expect(selectedIndex()).toBe(TOTAL - 1);
    expect(primary()).toHaveFocus();
    // 全页还是只有一个实心主按钮。
    expect(document.querySelectorAll('.ui-button--primary')).toHaveLength(1);
  });

  it('按住不放连着走；带 Shift、Ctrl、⌘、Alt 的不算', () => {
    renderQueue();
    press('j', { repeat: true });
    press('j', { repeat: true });
    expect(selectedIndex()).toBe(2);
    for (const init of [
      { shiftKey: true },
      { ctrlKey: true },
      { metaKey: true },
      { altKey: true },
    ])
      press('j', init);
    expect(selectedIndex()).toBe(2);
  });

  it('收起之后（没有选中的行）按 J 或 K 都从第一行开始', async () => {
    const user = userEvent.setup();
    renderQueue();
    const select = () =>
      rows()[0].querySelector<HTMLElement>('.cockpit-queue__select')!;
    await user.click(select());
    expect(selectedIndex()).toBe(-1);
    press('j');
    expect(selectedIndex()).toBe(0);
    await user.click(select());
    expect(selectedIndex()).toBe(-1);
    press('k');
    expect(selectedIndex()).toBe(0);
  });

  it('队列是空的：这几个键不归它管，原样放行', () => {
    renderQueue({
      board: { kind: 'disabled' },
      policy: { kind: 'absent' },
      inbox: [],
    });
    expect(screen.getByText('现在没有事等你。')).toBeInTheDocument();
    for (const key of ['j', 'k', 'h', 'Enter'])
      expect(press(key), key).toBe(true);
  });
});

describe('队列的键盘：Enter 去处理', () => {
  it('焦点没落在任何东西上：去处理选中的那一件', () => {
    const { handlers } = renderQueue();
    expect(press('Enter')).toBe(false);
    expect(handlers.onOpenPolicy).toHaveBeenCalledTimes(1);
    expect(handlers.onOpenTask).not.toHaveBeenCalled();
  });

  it('J 之后焦点在主按钮上：Enter 就是按钮自己的点击，不另外再处理一次', async () => {
    const user = userEvent.setup();
    const { handlers } = renderQueue();
    await user.keyboard('jj');
    expect(primary()).toHaveFocus();
    // 按钮上的 Enter 原样放行（返回 true＝没有被拦下）。
    expect(press('Enter')).toBe(true);
    expect(handlers.onOpenTask).not.toHaveBeenCalled();
    await user.keyboard('{Enter}');
    expect(handlers.onOpenTask).toHaveBeenCalledTimes(1);
    expect(handlers.onOpenTask).toHaveBeenCalledWith(TASK.T106);
  });

  it('工作台待办：打开原文', () => {
    const { handlers } = renderQueue();
    for (let step = 0; step < ORDER.length; step += 1) press('j');
    (document.activeElement as HTMLElement).blur();
    press('Enter');
    expect(handlers.onOpen).toHaveBeenCalledWith(
      data.workbench.inbox[0].source,
    );
  });

  it('焦点落在这一格外面的东西上：Enter 不算', () => {
    const { handlers } = renderQueue();
    const outside = document.createElement('div');
    outside.tabIndex = -1;
    document.body.append(outside);
    outside.focus();
    expect(press('Enter')).toBe(true);
    expect(handlers.onOpenPolicy).not.toHaveBeenCalled();
    // J 把焦点带回队列。
    press('j');
    expect(primary()).toHaveFocus();
    outside.remove();
  });

  it('没有选中的行：Enter 什么也不做', async () => {
    const user = userEvent.setup();
    const { handlers } = renderQueue();
    await user.click(
      rows()[0].querySelector<HTMLElement>('.cockpit-queue__select')!,
    );
    (document.activeElement as HTMLElement).blur();
    press('Enter');
    for (const handler of Object.values(handlers))
      expect(handler).not.toHaveBeenCalled();
  });
});

describe('队列的键盘：H 稍后', () => {
  it('把选中的那一件挪到队尾的「稍后再看」：总数、序号都不变，选中移到下一行', () => {
    const { storage } = renderQueue();
    const before = numbers();
    expect(laterList()).toBeNull();
    press('h');
    expect(laterTitles()).toEqual([PROPOSAL]);
    expect(titles()).toEqual([
      ...ORDER.slice(1),
      ...titles().slice(3, -1),
      PROPOSAL,
    ]);
    // 巨号不变：它还在等你。
    expect(countValue()).toHaveTextContent(String(TOTAL));
    expect(
      within(queue()).getByRole('heading', { level: 2 }),
    ).toHaveTextContent(`${TOTAL} 件事等你处理`);
    // 序号是它在队列里的位置，不重排：挪走的还是 01。
    expect(numbers()).toEqual([...before.slice(1), before[0]]);
    expect(
      within(queue()).getByRole('heading', { level: 3, name: /稍后再看/ }),
    ).toHaveTextContent('1 件');
    // 选中移到原来的第二行，焦点落在它的主按钮上。
    expect(selectedIndex()).toBe(0);
    expect(titles()[0]).toBe(ORDER[1]);
    expect(primary()).toHaveFocus();
    // 本机存储里只有一条记号，里面没有标题。
    const raw = storage.values.get(LATER_KEY)!;
    expect(Object.keys(JSON.parse(raw) as object)).toHaveLength(1);
    expect(raw).not.toContain('claude');
  });

  it('选回它再按一次 H：放回原位', () => {
    renderQueue();
    press('h');
    for (let step = 0; step < TOTAL; step += 1) press('j');
    expect(selectedIndex()).toBe(TOTAL - 1);
    expect(titles()[TOTAL - 1]).toBe(PROPOSAL);
    press('h');
    expect(laterList()).toBeNull();
    expect(titles().slice(0, ORDER.length)).toEqual(ORDER);
    expect(selectedIndex()).toBe(0);
    expect(primary()).toHaveFocus();
  });

  it('连按 H：一件接一件放走；全放走之后选中留在最后放走的那一件上', () => {
    renderQueue();
    for (let step = 0; step < TOTAL; step += 1) press('h');
    expect(laterTitles()).toHaveLength(TOTAL);
    expect(queue().querySelectorAll('ol')).toHaveLength(1);
    expect(selectedIndex()).toBe(TOTAL - 1);
    expect(countValue()).toHaveTextContent(String(TOTAL));
    // 再按一次：把选中的这一件放回。
    press('h');
    expect(laterTitles()).toHaveLength(TOTAL - 1);
  });

  it('放走的是最后一行：选中移到它前面那一行', () => {
    renderQueue();
    for (let step = 0; step < TOTAL; step += 1) press('j');
    const last = titles()[TOTAL - 1];
    const previous = titles()[TOTAL - 2];
    press('h');
    expect(laterTitles()).toEqual([last]);
    expect(titles()[selectedIndex()]).toBe(previous);
  });

  it('按住不放只认第一下；没有选中的行时不动', async () => {
    const user = userEvent.setup();
    renderQueue();
    press('h');
    press('h', { repeat: true });
    press('h', { repeat: true });
    expect(laterTitles()).toEqual([PROPOSAL]);
    await user.click(
      rows()[0].querySelector<HTMLElement>('.cockpit-queue__select')!,
    );
    expect(selectedIndex()).toBe(-1);
    press('h');
    expect(laterTitles()).toEqual([PROPOSAL]);
  });

  it('选中行的「稍后」按钮和 H 一样；在「稍后再看」里它写「放回队列」', async () => {
    const user = userEvent.setup();
    renderQueue();
    await user.click(
      screen.getByRole('button', { name: `${PROPOSAL}，稍后再看` }),
    );
    expect(laterTitles()).toEqual([PROPOSAL]);
    // 没选中的行没有这个按钮。
    expect(
      screen.queryByRole('button', { name: `${PROPOSAL}，放回队列` }),
    ).not.toBeInTheDocument();
    await user.click(
      rows().at(-1)!.querySelector<HTMLElement>('.cockpit-queue__select')!,
    );
    await user.click(
      screen.getByRole('button', { name: `${PROPOSAL}，放回队列` }),
    );
    expect(laterList()).toBeNull();
    expect(titles()[0]).toBe(PROPOSAL);
  });

  it('4 小时到了：自己回到原位，不用等别的事来重画', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    renderQueue();
    press('h');
    expect(laterTitles()).toEqual([PROPOSAL]);
    vi.mocked(Date.now).mockReturnValue(NOW + LATER_MS - 1000);
    act(() => {
      vi.advanceTimersByTime(LATER_MS - 1000);
    });
    expect(laterTitles()).toEqual([PROPOSAL]);
    vi.mocked(Date.now).mockReturnValue(NOW + LATER_MS + 100);
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(laterList()).toBeNull();
    expect(titles()[0]).toBe(PROPOSAL);
  });

  it('它有了新进展：回到原位', () => {
    const { rerender } = renderQueue();
    press('j');
    press('h');
    expect(laterTitles()).toEqual([titleOf(TASK.T093)]);
    const progressed: OverviewState = {
      kind: 'ready',
      overview: {
        ...data.overview,
        items: data.overview.items.map((item) =>
          item.id === TASK.T093 && item.progress
            ? {
                ...item,
                progress: {
                  ...item.progress,
                  lastProgressMs: item.progress.lastProgressMs + 60_000,
                },
              }
            : item,
        ),
      },
    };
    rerender({ board: progressed });
    expect(laterList()).toBeNull();
    expect(titles()).toContain(titleOf(TASK.T093));
  });

  it('别的标签页放过的：这里跟着挪', () => {
    const { later } = renderQueue();
    act(() => {
      later.later(`board:${TASK.T106}`, rowSince(TASK.T106));
    });
    expect(laterTitles()).toEqual([titleOf(TASK.T106)]);
  });
});

/** 一条黑板任务在队列里的起点：最后一次进展。 */
function rowSince(id: string): number {
  return data.overview.items.find((item) => item.id === id)!.progress!
    .lastProgressMs;
}

describe('「稍后」的淡出（有偏好查询能力的浏览器）', () => {
  beforeEach(() => {
    vi.stubGlobal('matchMedia', () => ({
      matches: false,
      addEventListener: () => {},
      removeEventListener: () => {},
    }));
  });

  it('先在原地淡出，走完才挪走；行里别的东西的动画结束不算', () => {
    renderQueue();
    press('h');
    expect(rows()[0]).toHaveAttribute('data-leave', 'true');
    expect(laterList()).toBeNull();
    fireEvent.animationEnd(primary()!);
    expect(rows()[0]).toHaveAttribute('data-leave', 'true');
    fireEvent.animationEnd(rows()[0]);
    expect(laterTitles()).toEqual([PROPOSAL]);
    expect(queue().querySelector('[data-leave]')).toBeNull();
    expect(selectedIndex()).toBe(0);
    expect(titles()[0]).toBe(ORDER[1]);
    expect(primary()).toHaveFocus();
  });

  it('动画没跑起来：最多等一小会儿照样挪走', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    renderQueue();
    press('h');
    expect(laterList()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(399);
    });
    expect(laterList()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(2);
    });
    expect(laterTitles()).toEqual([PROPOSAL]);
  });

  it('淡出还没走完就按下一个键：先落定再处理，不让人等动画', () => {
    renderQueue();
    press('h');
    press('h');
    // 第一件已经落定，第二件正在淡出。
    expect(laterTitles()).toEqual([PROPOSAL]);
    expect(rows()[0]).toHaveAttribute('data-leave', 'true');
    expect(titles()[0]).toBe(ORDER[1]);
    press('j');
    // 第二件也落定了；落定后选中的是第三件，J 再往下走一步到第四件。
    expect(laterTitles()).toEqual([PROPOSAL, ORDER[1]]);
    expect(queue().querySelector('[data-leave]')).toBeNull();
    expect(titles()[selectedIndex()]).toBe(ORDER[3]);
  });
});

describe('选中行就地展开的细节', () => {
  it('策略草案：扩大项和改动条目列在原地，不用红；黑板任务列写域；工作台待办不列', () => {
    renderQueue();
    const report = data.desired[0].report!;
    const detail = () => queue().querySelector<HTMLElement>('.cockpit-detail');
    const section = (label: string) =>
      within(detail()!)
        .getByRole('heading', { level: 3, name: new RegExp(`^${label}`) })
        .closest('section')!;
    expect(section('扩大')).toHaveTextContent(
      `扩大 ${report.expansions.length}`,
    );
    const expansionLines = within(section('扩大')).getAllByRole('listitem');
    expect(expansionLines).toHaveLength(report.expansions.length);
    expect(expansionLines[0]).toHaveTextContent(report.expansions[0].path);
    expect(expansionLines[0]).toHaveTextContent(report.expansions[0].note);
    // 方块只是辅助，读屏念「扩大」两个字。
    expect(within(expansionLines[0]).getByText('扩大')).toHaveClass('sr-only');
    expect(within(section('改动')).getAllByRole('listitem').length).toBe(
      Math.min(report.diff.entries.length, DETAIL_LIMIT),
    );
    // 细节是只读的：里面没有任何按钮。
    expect(within(detail()!).queryByRole('button')).toBeNull();

    press('j');
    const item = data.overview.items.find(({ id }) => id === TASK.T093)!;
    expect(
      within(section('写域'))
        .getAllByRole('listitem')
        .map((line) => line.textContent),
    ).toEqual(item.paths.slice(0, DETAIL_LIMIT));

    for (let step = 0; step < ORDER.length; step += 1) press('j');
    expect(rows()[selectedIndex()]).toHaveAttribute('data-source', 'inbox');
    expect(detail()).toBeNull();
  });
});

describe('进场和巨号滚动的开关', () => {
  it('三路都读完之后列表才打上 data-live：还在读时列出来的行不算新来的', () => {
    const { rerender } = renderQueue({ board: { kind: 'loading' } });
    const lists = () => Array.from(queue().querySelectorAll('ol'));
    expect(lists().length).toBeGreaterThan(0);
    for (const list of lists()) expect(list).not.toHaveAttribute('data-live');
    rerender({});
    for (const list of lists())
      expect(list).toHaveAttribute('data-live', 'true');
    // 手动刷新：又在读了，关掉；读完再开。
    rerender({ policy: { kind: 'loading' } });
    for (const list of lists()) expect(list).not.toHaveAttribute('data-live');
  });

  it('巨号从一个数变成另一个数才标滚动方向；「—」换成数、数换成「—」不滚', () => {
    const { rerender } = renderQueue();
    expect(countValue()).toHaveTextContent(String(TOTAL));
    expect(countValue()).not.toHaveAttribute('data-roll');
    const first = countValue();
    // 少了一件：从上面滚下来，而且换了一个节点。
    rerender({ inbox: data.workbench.inbox.slice(1) });
    expect(countValue()).toHaveTextContent(String(TOTAL - 1));
    expect(countValue()).toHaveAttribute('data-roll', 'down');
    expect(countValue()).not.toBe(first);
    // 多了一件：从下面滚上来。
    rerender({});
    expect(countValue()).toHaveAttribute('data-roll', 'up');
    // 手动刷新：先变成「—」，再回到数，都不滚。
    rerender({ board: { kind: 'loading' } });
    expect(countValue()).toHaveTextContent('—');
    expect(countValue()).not.toHaveAttribute('data-roll');
    rerender({ inbox: data.workbench.inbox.slice(1) });
    expect(countValue()).toHaveTextContent(String(TOTAL - 1));
    expect(countValue()).not.toHaveAttribute('data-roll');
  });
});

describe('队列底下的键位提示', () => {
  it('接上了全站监听才出现：列出这一格的键位，「全部快捷键」打开一览', async () => {
    const user = userEvent.setup();
    renderQueue();
    const hints = within(queue()).getByRole('list', {
      name: '这一格的键盘快捷键',
    });
    expect(
      within(hints)
        .getAllByRole('listitem')
        .map((item) => item.textContent),
    ).toEqual(['JK上下移动', '↵去处理', 'H稍后', 'G跳到别的视图']);
    await user.click(screen.getByRole('button', { name: /^全部快捷键/ }));
    expect(
      screen.getByRole('dialog', { name: '键盘快捷键' }),
    ).toBeInTheDocument();
  });

  it('单键快捷键关掉了：J／K／H 不响应，提示换成「已关闭」，入口还在', () => {
    const { preference } = renderQueue();
    act(() => preference.set(false));
    expect(within(queue()).getByText('单键快捷键已关闭')).toBeInTheDocument();
    expect(
      within(queue()).queryByRole('list', { name: '这一格的键盘快捷键' }),
    ).toBeNull();
    expect(
      screen.getByRole('button', { name: '全部快捷键' }),
    ).toBeInTheDocument();
    for (const key of ['j', 'h', 'Enter']) expect(press(key), key).toBe(true);
    expect(selectedIndex()).toBe(0);
    expect(laterList()).toBeNull();
    act(() => preference.set(true));
    press('j');
    expect(selectedIndex()).toBe(1);
  });

  it('没接上全站监听（单独渲染的视图）：不出提示，按键也没人管', () => {
    renderQueue({}, { live: false });
    expect(queue().querySelector('.cockpit-queue__keys')).toBeNull();
    expect(press('j')).toBe(true);
    expect(selectedIndex()).toBe(0);
  });
});
