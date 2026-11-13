import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import {
  LAST_SEEN_KEY,
  createSeenTracker,
  type SeenTracker,
} from '../../cockpit/lastSeen';
import type { OverviewRead } from '../../cockpit/useOverviewRead';
import type { PolicyPendingRead } from '../../cockpit/usePolicyPendingRead';
import { waitFraction } from '../../cockpit/wait';
import { REVALIDATED } from '../../live/autoRefresh';
import { executorOf, overviewLabels } from '../../collab/executorGrouping';
import { CockpitView } from '../../views/CockpitView';
import { CockpitHarness } from './CockpitHarness';
import {
  NOW,
  OVERVIEW_URL,
  fixtureData,
  fixtureFetch,
  policyReady,
  type FixtureData,
} from './cockpitFixtures';

const WAIT = { timeout: 4000 };
const OVERVIEW = '/api/v1/board/overview';
const DESIRED = '/api/v1/policy/desired';
const HEALTH = '/api/v1/health';
/** 假数据那份 AI 提案在队列里的标题。 */
const PROPOSAL = '能力集 claude';

type Overrides = Record<string, () => Response | Promise<Response>>;

const failed = (error: string, status: number) =>
  new Response(JSON.stringify({ error }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });

/** 内存里的假本机存储，给「上次查看」的记号用；每例各用各的，互不相干。 */
function memory(initial?: unknown) {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(LAST_SEEN_KEY, JSON.stringify(initial));
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}
const tracker = (storage: ReturnType<typeof memory> | null = null) =>
  createSeenTracker({ storage, now: Date.now });

beforeEach(() => {
  // 假数据的「此刻」是 2026-09-22 20:31：本机时钟拨到同一刻，草案等了多久、日期那一行才是定数。
  vi.spyOn(Date, 'now').mockReturnValue(NOW);
});

function renderCockpit(
  overrides: Overrides = {},
  options: { diagnostics?: number; seen?: SeenTracker } = {},
) {
  const data: FixtureData = fixtureData();
  const fetchMock = vi.fn(fixtureFetch(data.api, overrides));
  vi.stubGlobal('fetch', fetchMock);
  const handlers = { onOpen: vi.fn(), onShowDiagnostics: vi.fn() };
  const seen = options.seen ?? tracker();
  // 黑板汇总与策略草案由 App 各读一次交给驾驶舱；单独渲染驾驶舱时由外层代行那两份读取。
  const element = (refreshing: boolean) => (
    <CockpitHarness
      workbench={data.workbench}
      refreshing={refreshing}
      diagnostics={options.diagnostics ?? 0}
      seen={seen}
      {...handlers}
    />
  );
  const view = render(element(false));
  const requests = (prefix: string) =>
    fetchMock.mock.calls.filter(([input]) => String(input).startsWith(prefix))
      .length;
  return {
    data,
    handlers,
    requests,
    seen,
    rerender: (refreshing: boolean) => view.rerender(element(refreshing)),
  };
}

const queue = () => screen.getByRole('region', { name: '等你处理' });
const crew = () => screen.getByRole('complementary', { name: '谁在干活' });
const sinceCell = () => screen.getByRole('region', { name: '自上次查看以来' });
const readings = () => screen.getByRole('region', { name: '系统读数' });
/** 队列的行：按类名取，选中行里就地细节的小清单不算。 */
const rows = () =>
  Array.from(queue().querySelectorAll<HTMLElement>('.cockpit-queue__row'));
/** 巨号：对读屏隐藏，读屏读的是旁边二级标题里的数。 */
const count = () =>
  queue().querySelector('.cockpit-queue__count') as HTMLElement;
const heading = () => within(queue()).getByRole('heading', { level: 2 });
const titleOf = (data: FixtureData, id: string) =>
  data.overview.items.find((item) => item.id === id)!.title;
const reading = (label: string) =>
  within(readings()).getByText(label).closest('li') as HTMLElement;
const stat = (unit: string) =>
  within(sinceCell()).getByText(unit).closest('li') as HTMLElement;
/** 等三路都读完：巨号给出总数。 */
const settled = (total: number) =>
  waitFor(() => expect(count()).toHaveTextContent(String(total)), WAIT);

describe('驾驶舱：等你处理的队列', () => {
  it('三路并成一列：提案、黑板里等你的任务、工作台待办，统一编号，等得最久的在最上', async () => {
    const { data } = renderCockpit();
    const total = 4 + data.workbench.inbox.length;
    await settled(total);
    expect(
      screen.getByRole('heading', { level: 1, name: '驾驶舱' }),
    ).toBeInTheDocument();
    expect(rows()).toHaveLength(total);
    rows().forEach((row, index) =>
      expect(row.querySelector('.cockpit-queue__n')).toHaveTextContent(
        String(index + 1).padStart(2, '0'),
      ),
    );
    // 提案 09:12 提交，排第一；黑板三条按最后一次进展的先后；工作台待办不带时间，接在后面。
    const names = [
      `${PROPOSAL}，在策略页处理`,
      ...[TASK.T093, TASK.T106, TASK.T108].map(
        (id) => `${titleOf(data, id)}，在协作页查看`,
      ),
      ...data.workbench.inbox.map((action) => `${action.title}，查看原文`),
    ];
    names.forEach((name, index) =>
      expect(
        within(rows()[index]).getByRole('button', { name }),
      ).toBeInTheDocument(),
    );
    expect(
      rows()
        .slice(0, 4)
        .map((row) => row.querySelector('.cockpit-queue__kind')?.textContent),
    ).toEqual(['AI 提案', '待修订', '待验收', '待验收']);
    // 巨号和标题同一行：巨号对读屏隐藏，标题把数读出来。
    expect(count()).toHaveAttribute('aria-hidden', 'true');
    expect(count()).toHaveAttribute('data-state', 'waiting');
    expect(heading()).toHaveAccessibleName(`${total} 件事等你处理`);
    expect(
      within(queue()).getByText('最久的已等 11 时 19 分'),
    ).toBeInTheDocument();
    expect(within(queue()).getByText('等了多久算到 20:31')).toBeInTheDocument();
  });

  it('缺省选中第一行：多一行事实和主按钮；点别的行的标题换过去，再点一次收起', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    const user = userEvent.setup();
    const select = (index: number) =>
      rows()[index].querySelector('.cockpit-queue__select') as HTMLElement;
    const expanded = () =>
      rows().map((_, index) => select(index).getAttribute('aria-expanded'));
    expect(expanded().indexOf('true')).toBe(0);
    expect(expanded().filter((value) => value === 'true')).toHaveLength(1);
    // 选中行：一行校验结果，一个实心主按钮；没有行尾的文字链接。
    expect(within(rows()[0]).getByText('校验')).toBeInTheDocument();
    expect(within(rows()[0]).getByText('扩大 3 项')).toBeInTheDocument();
    expect(
      within(rows()[0]).getByRole('button', {
        name: `${PROPOSAL}，在策略页处理`,
      }),
    ).toHaveClass('ui-button--primary');
    expect(rows()[0].querySelector('.cockpit-queue__link')).toBeNull();
    // 其余行：行尾一个文字链接，没有主按钮。
    expect(rows()[2].querySelector('.cockpit-queue__link')).not.toBeNull();
    expect(rows()[2].querySelector('.ui-button--primary')).toBeNull();
    // 全页只有这一个实心主按钮。
    expect(document.querySelectorAll('.ui-button--primary')).toHaveLength(1);

    await user.click(select(2));
    expect(expanded().indexOf('true')).toBe(2);
    expect(within(rows()[2]).getByText('进度')).toBeInTheDocument();
    expect(within(rows()[2]).getByText('工具调用 88 次')).toBeInTheDocument();
    expect(
      within(rows()[2]).getByRole('button', {
        name: `${titleOf(data, TASK.T106)}，在协作页查看`,
      }),
    ).toHaveClass('ui-button--primary');
    expect(rows()[0].querySelector('.cockpit-queue__link')).not.toBeNull();
    expect(within(rows()[0]).queryByText('校验')).not.toBeInTheDocument();
    expect(document.querySelectorAll('.ui-button--primary')).toHaveLength(1);

    await user.click(select(2));
    expect(expanded()).not.toContain('true');
    expect(document.querySelectorAll('.ui-button--primary')).toHaveLength(0);
  });

  it('去处理：黑板任务去协作页并带上那条任务，提案去策略页「待处理」里的那一份草案，工作台待办打开原文', async () => {
    const { data, handlers } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    const user = userEvent.setup();
    await user.click(
      screen.getByRole('button', {
        name: `${titleOf(data, TASK.T106)}，在协作页查看`,
      }),
    );
    expect(window.location.hash).toBe(`#view=collab&item=${TASK.T106}`);
    await user.click(
      screen.getByRole('button', { name: `${PROPOSAL}，在策略页处理` }),
    );
    // 位置里只有标签名和草案号，草案正文不进 URL。
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${data.desired[0].id}`,
    );
    const [first] = data.workbench.inbox;
    await user.click(
      screen.getByRole('button', { name: `${first.title}，查看原文` }),
    );
    expect(handlers.onOpen).toHaveBeenCalledWith(first.source);
  });

  it('等了多久：一行文字加一条对数刻度条；工作台待办不带时间，写「—」、不画条', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    const waited = (index: number) =>
      rows()[index].querySelector('.cockpit-queue__waited') as HTMLElement;
    const bar = (index: number) =>
      rows()[index].querySelector('.cockpit-wait') as HTMLElement | null;
    // T-093 最后进展 10:03，汇总生成 20:31。
    expect(waited(1)).toHaveTextContent('已等 10 时 28 分');
    const ms =
      NOW -
      data.overview.items.find((item) => item.id === TASK.T093)!.progress!
        .lastProgressMs;
    expect(bar(1)!.style.getPropertyValue('--cockpit-wait')).toBe(
      `${(waitFraction(ms) * 100).toFixed(1)}%`,
    );
    expect(bar(1)).toHaveAttribute('aria-hidden', 'true');
    // 提案从提交（09:12）算到读到列表的那一刻。
    expect(waited(0)).toHaveTextContent('已等 11 时 19 分');
    expect(waited(4)).toHaveTextContent('—，不知道等了多久');
    expect(bar(4)).toBeNull();
    expect(within(queue()).getByText('等了多久')).toBeInTheDocument();
  });
});

describe('驾驶舱：三路各自成败', () => {
  it('黑板未启用（404）：那一路确实没有，队列照常、不报错；黑板相关的格子各写原因', async () => {
    const { data } = renderCockpit({
      [OVERVIEW]: () => new Response(null, { status: 404 }),
    });
    const total = 1 + data.workbench.inbox.length;
    await settled(total);
    expect(within(crew()).getByText('黑板未启用')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(rows()).toHaveLength(total);
    expect(
      within(rows()[0]).getByRole('button', {
        name: `${PROPOSAL}，在策略页处理`,
      }),
    ).toBeInTheDocument();
    for (const label of ['黑板在跑', '执行'])
      expect(reading(label)).toHaveAttribute('title', '黑板未启用');
    expect(within(reading('可读项目')).getByText('8')).toBeInTheDocument();
    for (const unit of ['个任务完成', '个新建任务']) {
      expect(stat(unit)).toHaveAttribute('title', '黑板未启用。');
      expect(within(stat(unit)).getByText('—')).toBeInTheDocument();
    }
    // 没有黑板汇总的生成时刻：「等了多久」算到读到策略草案的那一刻。
    expect(within(queue()).getByText('等了多久算到 20:31')).toBeInTheDocument();
  });

  it('黑板读取失败：总数不可知、右栏报错并可重试；重试只重读黑板汇总', async () => {
    let failing = true;
    const { data, requests } = renderCockpit({
      [OVERVIEW]: () =>
        failing
          ? failed('黑板许可已满。', 429)
          : fixtureFetch(data.api)(OVERVIEW_URL),
    });
    const alert = await within(crew()).findByRole('alert', undefined, WAIT);
    expect(alert).toHaveTextContent('黑板汇总读取失败');
    expect(alert).toHaveTextContent('黑板许可已满。');
    expect(screen.getAllByRole('alert')).toHaveLength(1);
    // 总数不可知就不给数；已经读到的两路照常列出，并说明缺了哪一路。
    await waitFor(() =>
      expect(rows()).toHaveLength(1 + data.workbench.inbox.length),
    );
    expect(count()).toHaveTextContent('—');
    expect(count()).toHaveAttribute('data-state', 'unknown');
    // 隐藏的那半句和看得见的那半句之间有没有空格，各家读屏名算法不一样（jsdom 这套会加），不核这一格。
    expect(heading()).toHaveAccessibleName(/^不知道有几\s?件事等你处理$/);
    expect(
      within(queue()).getByText(
        '黑板汇总读取失败，等你的黑板任务没列出来；可在右边重试。',
      ),
    ).toHaveAttribute('role', 'status');
    const health = requests(HEALTH);
    failing = false;
    await userEvent
      .setup()
      .click(within(alert).getByRole('button', { name: '重试黑板汇总' }));
    await settled(4 + data.workbench.inbox.length);
    expect(requests(OVERVIEW)).toBe(2);
    expect(requests(DESIRED)).toBe(1);
    expect(requests(HEALTH)).toBe(health);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('黑板读取中：右栏显示读取占位，巨号不给数，队列先列已经读到的', async () => {
    const { data } = renderCockpit({
      [OVERVIEW]: () => new Promise<Response>(() => {}),
    });
    expect(within(crew()).getByRole('status')).toHaveTextContent(
      '正在读取黑板汇总…',
    );
    expect(
      within(queue()).getByText('正在读取黑板里等你的任务…'),
    ).toBeInTheDocument();
    expect(count()).toHaveTextContent('—');
    await waitFor(() =>
      expect(rows()).toHaveLength(1 + data.workbench.inbox.length),
    );
    expect(count()).toHaveTextContent('—');
  });

  it('策略草案读取失败：总数不可知、队列里就地说明并可重试；重试只重读策略草案', async () => {
    let failing = true;
    const { data, requests } = renderCockpit({
      [DESIRED]: () =>
        failing
          ? failed('策略服务暂时不可用。', 503)
          : fixtureFetch(data.api)(DESIRED),
    });
    const retry = await within(queue()).findByRole(
      'button',
      { name: '重试策略草案' },
      WAIT,
    );
    expect(retry.closest('[role="status"]')).toHaveTextContent(
      '策略草案读取失败，等你的提案和草案没列出来：策略服务暂时不可用。',
    );
    await waitFor(() =>
      expect(rows()).toHaveLength(3 + data.workbench.inbox.length),
    );
    expect(count()).toHaveTextContent('—');
    // 这不是黑板的错：右栏照常，不出报错块。
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(stat('份新提案和草案')).toHaveAttribute(
      'title',
      '策略草案读取失败：策略服务暂时不可用。',
    );
    failing = false;
    await userEvent.setup().click(retry);
    await settled(4 + data.workbench.inbox.length);
    expect(requests(DESIRED)).toBe(2);
    expect(requests(OVERVIEW)).toBe(1);
    expect(
      within(queue()).queryByRole('button', { name: '重试策略草案' }),
    ).not.toBeInTheDocument();
  });

  it('本入口没装策略平面（404）：那一路确实没有，总数照给，不当成失败', async () => {
    const { data } = renderCockpit({
      [DESIRED]: () =>
        new Response(
          JSON.stringify({
            code: 'policy_plane_absent',
            error: '本入口未装配策略平面。',
          }),
          { status: 404, headers: { 'Content-Type': 'application/json' } },
        ),
    });
    await settled(3 + data.workbench.inbox.length);
    expect(rows()).toHaveLength(3 + data.workbench.inbox.length);
    expect(within(queue()).queryByRole('status')).not.toBeInTheDocument();
    expect(stat('份新提案和草案')).toHaveAttribute(
      'title',
      '本入口没有接入策略平面。',
    );
  });

  it('策略草案读取中：巨号不给数，并说明还在读哪一路', async () => {
    const { data } = renderCockpit({
      [DESIRED]: () => new Promise<Response>(() => {}),
    });
    await waitFor(
      () => expect(rows()).toHaveLength(3 + data.workbench.inbox.length),
      WAIT,
    );
    expect(count()).toHaveTextContent('—');
    expect(
      within(queue()).getByText('正在读取策略里等你的提案和草案…'),
    ).toBeInTheDocument();
  });

  it('两路都在读时各有一句；黑板先读完，策略那一句还是原来那个节点，不被换掉', async () => {
    let release = (): void => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const { data } = renderCockpit({
      [OVERVIEW]: async () => {
        await held;
        return fixtureFetch(data.api)(OVERVIEW_URL);
      },
      [DESIRED]: () => new Promise<Response>(() => {}),
    });
    const boardNote = '正在读取黑板里等你的任务…';
    const policyNote =
      within(queue()).getByText('正在读取策略里等你的提案和草案…');
    expect(within(queue()).getByText(boardNote)).toBeInTheDocument();
    release();
    await waitFor(
      () =>
        expect(within(queue()).queryByText(boardNote)).not.toBeInTheDocument(),
      WAIT,
    );
    // 说明按来源取键：前一句没了，后一句不挪位置、不重建（按序号取键时这里会换成另一个节点，读屏会把同一句话再念一遍）。
    expect(policyNote).toBeInTheDocument();
    expect(within(queue()).getByText('正在读取策略里等你的提案和草案…')).toBe(
      policyNote,
    );
    expect(count()).toHaveTextContent('—');
  });

  it('三路都没有事：照实说「现在没有事等你」，巨号是 0、不标红', async () => {
    const data = fixtureData();
    vi.stubGlobal(
      'fetch',
      vi.fn(
        fixtureFetch(data.api, {
          [OVERVIEW]: () => new Response(null, { status: 404 }),
          [DESIRED]: () =>
            new Response(JSON.stringify({ code: 'policy_plane_absent' }), {
              status: 404,
              headers: { 'Content-Type': 'application/json' },
            }),
        }),
      ),
    );
    render(
      <CockpitHarness
        workbench={{ ...data.workbench, inbox: [] }}
        refreshing={false}
        diagnostics={0}
        seen={tracker()}
        onOpen={vi.fn()}
        onShowDiagnostics={vi.fn()}
      />,
    );
    expect(
      await within(queue()).findByText('现在没有事等你。', undefined, WAIT),
    ).toBeInTheDocument();
    expect(count()).toHaveTextContent('0');
    expect(count()).toHaveAttribute('data-state', 'clear');
    expect(within(queue()).queryByRole('list')).not.toBeInTheDocument();
    expect(within(queue()).queryByText('等了多久')).not.toBeInTheDocument();
  });
});

describe('驾驶舱：谁在干活', () => {
  it('每个执行者一条 24 小时泳道，停下等你的排最前；下接最近动态', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    const labels = overviewLabels(data.overview);
    const executor = (id: string) =>
      labels.get(
        executorOf(data.overview.items.find((item) => item.id === id)!).key,
      )!;
    expect(
      within(crew()).getByText('2 个在跑 · 2 个停下等你'),
    ).toBeInTheDocument();
    const list = crew().querySelector('.cockpit-crew__list') as HTMLElement;
    const lines = within(list).getAllByRole('listitem');
    expect(
      lines.map(
        (line) => line.querySelector('.cockpit-crew__name')?.textContent,
      ),
    ).toEqual(
      [TASK.T093, TASK.T106, TASK.T130, TASK.T109, TASK.T046].map(executor),
    );
    expect(lines.map((line) => line.getAttribute('data-state'))).toEqual([
      'waiting',
      'waiting',
      'running',
      'running',
      'idle',
    ]);
    expect(
      lines.map(
        (line) => line.querySelector('.cockpit-crew__status')?.textContent,
      ),
    ).toEqual([
      '等你 10 时 28 分',
      '等你 5 时 28 分',
      '进行中 · 34 分前',
      '进行中 · 2 时前',
      '休眠 · 4 天前',
    ]);
    expect(within(lines[0]).getByText('另 1 条')).toBeInTheDocument();
    // 泳道是一幅图：停下等你的那条有红段，在跑的没有。
    const lane = (index: number) => within(lines[index]).getByRole('img');
    expect(lane(0)).toHaveAccessibleName(
      /^Codex · \S+，最近 24 小时：干活 \d+ 段，现在停下等你$/,
    );
    expect(lane(0).querySelectorAll('.ui-lanes__block--owner')).toHaveLength(1);
    expect(lane(1).querySelectorAll('.ui-lanes__block--owner')).toHaveLength(2);
    expect(lane(2).querySelectorAll('.ui-lanes__block--owner')).toHaveLength(0);
    expect(
      lane(2).querySelectorAll('.ui-lanes__block--work').length,
    ).toBeGreaterThan(0);
    expect(lane(4)).toHaveAccessibleName(/没有干活的记录$/);
    expect(lane(4).querySelectorAll('rect')).toHaveLength(0);
    // 刻度：四格，每格 6 小时。
    expect(
      [...crew().querySelectorAll('.cockpit-crew__scale span')].map(
        (tick) => tick.textContent,
      ),
    ).toEqual(['昨天 21:00', '03:00', '09:00', '15:00']);

    const feed = within(crew()).getByRole('region', { name: '最近动态' });
    expect(within(feed).getByText('截至 20:31')).toBeInTheDocument();
    expect(within(feed).getAllByRole('listitem')).toHaveLength(5);
    await userEvent
      .setup()
      .click(within(feed).getByRole('button', { name: '去协作页看全部' }));
    expect(window.location.hash).toBe('#view=collab');
  });
});

describe('驾驶舱：自上次查看以来', () => {
  it('这台浏览器还没有记录：先按最近 24 小时算并照实说明；拿不到的数写「—」和原因', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    expect(
      within(sinceCell()).getByText(
        '这台浏览器还没有上次查看的记录，先按最近 24 小时算',
      ),
    ).toBeInTheDocument();
    expect(stat('个任务完成')).toHaveTextContent('1个任务完成T-021');
    expect(stat('个新建任务')).toHaveTextContent('0个新建任务黑板上没有新建的');
    expect(stat('份新提案和草案')).toHaveTextContent(
      '1份新提案和草案其中 1 份是 AI 提的',
    );
    expect(stat('次写入')).toHaveTextContent(
      '—次写入没有上次查看时的提交水位，没法比。',
    );
  });

  it('有上次查看的记录：从那一刻数起，写入次数是提交水位之差', async () => {
    const committed = BigInt(
      fixtureData().workbench.freshness!.committed_through!,
    );
    const { data } = renderCockpit(
      {},
      {
        seen: tracker(
          memory({
            at: new Date(2026, 8, 22, 8, 0).getTime(),
            committed: String(committed - 5n),
          }),
        ),
      },
    );
    await settled(4 + data.workbench.inbox.length);
    expect(
      within(sinceCell()).getByText('08:00 起，12 时 31 分'),
    ).toBeInTheDocument();
    expect(stat('次写入')).toHaveTextContent('5次写入网页编辑和 AI 写回');
    expect(stat('个任务完成')).toHaveTextContent('1个任务完成T-021');
  });

  it('看到黑板读数才记一笔，离开页面时写下：时刻是汇总生成时刻，外加提交水位', async () => {
    const storage = memory();
    const { data, seen } = renderCockpit({}, { seen: tracker(storage) });
    await settled(4 + data.workbench.inbox.length);
    expect(storage.values.size).toBe(0);
    seen.leave();
    expect(JSON.parse(storage.values.get(LAST_SEEN_KEY)!)).toEqual({
      at: Date.parse(data.overview.generatedAt),
      committed: data.workbench.freshness!.committed_through,
    });
  });

  it('黑板还在读时不记：那一刻你没看到黑板的数', async () => {
    const storage = memory();
    const { data, seen } = renderCockpit(
      { [OVERVIEW]: () => new Promise<Response>(() => {}) },
      { seen: tracker(storage) },
    );
    await waitFor(() =>
      expect(rows()).toHaveLength(1 + data.workbench.inbox.length),
    );
    seen.leave();
    expect(storage.values.size).toBe(0);
  });
});

describe('驾驶舱：系统读数', () => {
  it('八格读数来自接口；要看的那一格标出来，拿不到的显示「—」并把原因写进格子', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    await within(reading('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(within(readings()).getAllByRole('listitem')).toHaveLength(8);
    expect(within(reading('黑板在跑')).getByText('7')).toBeInTheDocument();
    expect(within(reading('执行')).getByText('3')).toBeInTheDocument();
    expect(within(reading('会话')).getByText('今日已复核')).toBeInTheDocument();
    // 副本没追平、同步降级：这两格要看（黑底反白，不用红）。
    for (const label of ['副本新鲜度', '每轮同步']) {
      expect(reading(label)).toHaveAttribute('data-alert', 'true');
      expect(reading(label)).toHaveClass('ui-on-ink');
    }
    expect(reading('会话')).not.toHaveAttribute('data-alert');
    expect(within(readings()).getByText('8 项里 2 项要看')).toBeInTheDocument();
    expect(within(reading('未关闭 BUG')).getByText('—')).toBeInTheDocument();
    expect(reading('未关闭 BUG')).toHaveAttribute(
      'title',
      expect.stringContaining('不可得'),
    );
    expect(
      within(readings()).getByText('2026-09-22 周二 · 副本代次 243'),
    ).toBeInTheDocument();
  });

  it('本机健康读不到时只有取自健康的两格写原因，其余照常', async () => {
    const { data } = renderCockpit({
      [HEALTH]: () => failed('本机未装配健康证据来源。', 404),
    });
    await settled(4 + data.workbench.inbox.length);
    for (const label of ['每轮同步', '会话'])
      await waitFor(
        () =>
          expect(reading(label)).toHaveAttribute(
            'title',
            '本机健康读取失败：本机未装配健康证据来源。',
          ),
        WAIT,
      );
    expect(within(reading('黑板在跑')).getByText('7')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('数据提示只报条数，入口交给顶栏状态浮层；没有提示时不出入口', async () => {
    const { data, handlers } = renderCockpit({}, { diagnostics: 7 });
    await settled(4 + data.workbench.inbox.length);
    const entry = within(readings()).getByRole('button', {
      name: '7 条数据提示，查看数据提示',
    });
    expect(entry).toHaveTextContent('7 条数据提示');
    await userEvent.setup().click(entry);
    expect(handlers.onShowDiagnostics).toHaveBeenCalledTimes(1);
  });

  it('没有数据提示时不出入口', async () => {
    const { data } = renderCockpit();
    await settled(4 + data.workbench.inbox.length);
    expect(
      within(readings()).queryByRole('button', { name: /数据提示/ }),
    ).not.toBeInTheDocument();
  });
});

describe('驾驶舱：刷新', () => {
  it('一轮刷新结束后连带重读黑板汇总、策略草案与本机健康', async () => {
    const { data, rerender, requests } = renderCockpit();
    const total = 4 + data.workbench.inbox.length;
    await settled(total);
    await within(reading('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(1);
    expect(requests(DESIRED)).toBe(1);
    expect(requests(HEALTH)).toBe(1);
    rerender(true);
    expect(screen.getByText(/（刷新中）$/)).toBeInTheDocument();
    rerender(false);
    // 重读期间不拿上一次的结论冒充当前状态。
    expect(count()).toHaveTextContent('—');
    await settled(total);
    await within(reading('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(2);
    expect(requests(DESIRED)).toBe(2);
    expect(requests(HEALTH)).toBe(2);
  });
});

/** 路径以 prefix 开头的请求数。 */
function countSent(calls: readonly unknown[][], prefix: string) {
  return calls.filter(([input]) => String(input).startsWith(prefix)).length;
}

describe('驾驶舱：黑板汇总与策略草案由 App 各读一次交进来（T-162、T-195）', () => {
  /** 交进来的读取都带一个安静重读（自动刷新用，T-196）；这里没有调度器，不会被调到。 */
  const revalidate = () => Promise.resolve(REVALIDATED);

  /** 直接渲染驾驶舱、交两份现成的读取；返回按路径前缀数请求的函数。 */
  function renderWithReads(
    data: FixtureData,
    overview: OverviewRead,
    policy: PolicyPendingRead,
  ) {
    const fetchMock = vi.fn(fixtureFetch(data.api));
    vi.stubGlobal('fetch', fetchMock);
    render(
      <CockpitView
        workbench={data.workbench}
        refreshing={false}
        overview={overview}
        policy={policy}
        diagnostics={0}
        onOpen={vi.fn()}
        onShowDiagnostics={vi.fn()}
        seen={tracker()}
      />,
    );
    return (prefix: string) => countSent(fetchMock.mock.calls, prefix);
  }

  it('只用交进来的那两份，自己不另读；本机健康照旧自己读', async () => {
    const data = fixtureData();
    const reloadBoard = vi.fn();
    const reloadPolicy = vi.fn();
    const requests = renderWithReads(
      data,
      {
        state: { kind: 'ready', overview: data.overview },
        reload: reloadBoard,
        revalidate,
      },
      { state: policyReady(data.desired), reload: reloadPolicy, revalidate },
    );
    // 交进来的就是读完的：首屏即给数，不经过读取中。
    expect(count()).toHaveTextContent(String(4 + data.workbench.inbox.length));
    expect(rows()).toHaveLength(4 + data.workbench.inbox.length);
    await within(reading('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(HEALTH)).toBe(1);
    expect(requests(OVERVIEW)).toBe(0);
    expect(requests(DESIRED)).toBe(0);
    expect(reloadBoard).not.toHaveBeenCalled();
    expect(reloadPolicy).not.toHaveBeenCalled();
  });

  it('两处「重试」各调交进来的 reload，自己不发请求', async () => {
    const data = fixtureData();
    const reloadBoard = vi.fn();
    const reloadPolicy = vi.fn();
    const requests = renderWithReads(
      data,
      {
        state: { kind: 'error', message: '黑板许可已满。' },
        reload: reloadBoard,
        revalidate,
      },
      {
        state: { kind: 'error', message: '策略服务暂时不可用。' },
        reload: reloadPolicy,
        revalidate,
      },
    );
    expect(count()).toHaveTextContent('—');
    const user = userEvent.setup();
    await user.click(
      within(crew()).getByRole('button', { name: '重试黑板汇总' }),
    );
    expect(reloadBoard).toHaveBeenCalledTimes(1);
    expect(reloadPolicy).not.toHaveBeenCalled();
    await user.click(
      within(queue()).getByRole('button', { name: '重试策略草案' }),
    );
    expect(reloadPolicy).toHaveBeenCalledTimes(1);
    expect(reloadBoard).toHaveBeenCalledTimes(1);
    // 等本机健康读完再收尾，免得更新落在测试结束之后。
    await within(reading('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(0);
    expect(requests(DESIRED)).toBe(0);
  });
});
