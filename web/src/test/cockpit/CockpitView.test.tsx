import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import type { TopologyDiagramProps } from '../../collab/TopologyDiagram';
import type { OverviewRead } from '../../cockpit/useOverviewRead';
import { CockpitView } from '../../views/CockpitView';
import { CockpitHarness } from './CockpitHarness';
import {
  fixtureData,
  fixtureFetch,
  OVERVIEW_URL,
  type FixtureData,
} from './cockpitFixtures';

/*
 * 拓扑图归协作线（B2）：这里只核驾驶舱交给它的参数，不依赖它画成什么样。
 */
const topology = vi.hoisted(() => ({ calls: [] as TopologyDiagramProps[] }));
vi.mock('../../collab/TopologyDiagram', () => ({
  TopologyDiagram: (props: TopologyDiagramProps) => {
    topology.calls.push(props);
    return <figure aria-label={props.title} data-testid="topology" />;
  },
}));

const WAIT = { timeout: 4000 };
const OVERVIEW = '/api/v1/board/overview';
const HEALTH = '/api/v1/health';

function renderCockpit(
  overrides: Record<string, () => Response | Promise<Response>> = {},
  diagnostics = 0,
) {
  topology.calls.length = 0;
  const data: FixtureData = fixtureData();
  const fetchMock = vi.fn(fixtureFetch(data.api, overrides));
  vi.stubGlobal('fetch', fetchMock);
  const handlers = {
    onRefresh: vi.fn(),
    onOpen: vi.fn(),
    onEnterProject: vi.fn(),
    onViewProjects: vi.fn(),
    onShowDiagnostics: vi.fn(),
  };
  // 黑板汇总由 App 读一次交给驾驶舱（T-162）；单独渲染驾驶舱时由外层代行那一份读取。
  const element = (refreshing: boolean) => (
    <CockpitHarness
      workbench={data.workbench}
      refreshing={refreshing}
      filters={null}
      actions={data.workbench.inbox}
      projects={data.workbench.projects}
      diagnostics={diagnostics}
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
    rerender: (refreshing: boolean) => view.rerender(element(refreshing)),
  };
}

const region = (name: string) => screen.getByRole('region', { name });
const titleCell = (label: string) =>
  within(screen.getByRole('region', { name: '图纸标题栏' }))
    .getByText(label)
    .closest('li') as HTMLElement;
const titleOf = (data: FixtureData, id: string) =>
  data.overview.items.find((item) => item.id === id)!.title;
/** 巨号的数字与下面的小字。 */
const hero = () =>
  screen.getByText('件事等你处理').previousElementSibling as HTMLElement;
const heroNote = () =>
  screen.getByText('件事等你处理').nextElementSibling as HTMLElement;

describe('驾驶舱：待我动作与拓扑联动', () => {
  it('统一编号：黑板里等你的在前、圈号红色实心；工作台待办在后、圈号黑色空心', async () => {
    const { data } = renderCockpit();
    const pending = region('待我动作');
    await within(pending).findByRole(
      'button',
      { name: `${titleOf(data, TASK.T093)}，在协作页查看` },
      WAIT,
    );
    const rows = within(pending).getAllByRole('listitem');
    expect(rows).toHaveLength(3 + data.workbench.inbox.length);
    rows.forEach((row, index) => {
      const circle = row.querySelector('.ui-circled')!;
      expect(circle).toHaveTextContent(String(index + 1));
      expect(circle).toHaveAttribute(
        'data-filled',
        index < 3 ? 'true' : 'false',
      );
    });
    [TASK.T093, TASK.T106, TASK.T108].forEach((id, index) =>
      expect(
        within(rows[index]).getByRole('button', {
          name: `${titleOf(data, id)}，在协作页查看`,
        }),
      ).toBeInTheDocument(),
    );
    data.workbench.inbox.forEach((action, index) =>
      expect(
        within(rows[3 + index]).getByRole('button', {
          name: `${action.title}，查看原文`,
        }),
      ).toBeInTheDocument(),
    );
    expect(within(pending).getByText('共 8 项')).toBeInTheDocument();
    // 巨号：待我动作总数补成两位，读屏读自然数；最久与上图条数写在小字里。
    expect(within(hero()).getByText('08')).toHaveAttribute(
      'aria-hidden',
      'true',
    );
    expect(within(hero()).getByText('8')).toHaveClass('sr-only');
    expect(heroNote()).toHaveTextContent('最久不到 1 天，3 件标在拓扑图上');
  });

  it('交给拓扑缩略的标记只来自黑板条目，编号与待我动作一致；缩略图至多 6 条', async () => {
    const { data } = renderCockpit();
    await screen.findByTestId('topology', undefined, WAIT);
    const props = topology.calls.at(-1)!;
    expect(props).toMatchObject({
      mode: 'mini',
      maxTasks: 6,
      showDormant: true,
    });
    expect(props.markers).toEqual([
      { n: 1, target: { taskId: TASK.T093 } },
      { n: 2, target: { taskId: TASK.T106 } },
      { n: 3, target: { taskId: TASK.T108 } },
    ]);
    const boardIds = new Set(data.overview.items.map((item) => item.id));
    const inboxIds = new Set(data.workbench.inbox.map((action) => action.id));
    for (const marker of props.markers!) {
      expect('taskId' in marker.target).toBe(true);
      const id = 'taskId' in marker.target ? marker.target.taskId : '';
      expect(boardIds.has(id)).toBe(true);
      expect(inboxIds.has(id)).toBe(false);
    }
    expect(props.items).toHaveLength(6);
    expect(props.now).toBe(Date.parse(data.overview.generatedAt));
    // 第 7 条未结束任务没画上：标题行给出去协作页的入口。
    const more = within(region('协作拓扑，今天')).getByRole('button', {
      name: '另 1 条，去协作页',
    });
    await userEvent.setup().click(more);
    expect(window.location.hash).toBe('#view=collab');
  });

  it('悬停或聚焦实心圈的那一行，缩略图同步选中那条任务；点开去协作页看它', async () => {
    const { data } = renderCockpit();
    const row = await within(region('待我动作')).findByRole(
      'button',
      { name: `${titleOf(data, TASK.T106)}，在协作页查看` },
      WAIT,
    );
    fireEvent.mouseEnter(row);
    expect(topology.calls.at(-1)!.selectedTaskId).toBe(TASK.T106);
    fireEvent.mouseLeave(row);
    expect(topology.calls.at(-1)!.selectedTaskId).toBeNull();
    fireEvent.focus(row);
    expect(topology.calls.at(-1)!.selectedTaskId).toBe(TASK.T106);
    fireEvent.blur(row);
    await userEvent.setup().click(row);
    expect(window.location.hash).toBe(`#view=collab&item=${TASK.T106}`);
  });

  it('工作台待办点开原文', async () => {
    const { data, handlers } = renderCockpit();
    const [first] = data.workbench.inbox;
    await userEvent
      .setup()
      .click(
        await screen.findByRole(
          'button',
          { name: `${first.title}，查看原文` },
          WAIT,
        ),
      );
    expect(handlers.onOpen).toHaveBeenCalledWith(first.source);
  });
});

describe('驾驶舱：项目明细', () => {
  it('只列前五行，「全部 N 个项目」写明总数并交给项目视图；项目名进入运作', async () => {
    const { data, handlers } = renderCockpit({}, 7);
    const projects = region('项目明细');
    const rows = within(projects).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(5);
    expect(within(rows[0]).getByText('01')).toBeInTheDocument();
    const all = within(projects).getByRole('button', {
      name: `全部 ${data.workbench.projects.length} 个项目，打开项目`,
    });
    expect(all).toHaveTextContent(
      `全部 ${data.workbench.projects.length} 个项目`,
    );
    const user = userEvent.setup();
    await user.click(all);
    expect(handlers.onViewProjects).toHaveBeenCalledTimes(1);
    const name = within(rows[0]).getByRole('button', {
      name: /，进入项目运作$/,
    });
    await user.click(name);
    expect(handlers.onEnterProject).toHaveBeenCalledTimes(1);
    // 数据提示只报条数，入口交给状态浮层。
    expect(
      within(projects).getByText('当前快照有 7 条数据提示：缺失信息保持未知。'),
    ).toBeInTheDocument();
    await user.click(
      within(projects).getByRole('button', { name: '查看数据提示' }),
    );
    expect(handlers.onShowDiagnostics).toHaveBeenCalledTimes(1);
  });
});

describe('驾驶舱：标题栏', () => {
  it('每格读数来自接口；拿不到的显示「—」并把原因写进格子', async () => {
    renderCockpit();
    await within(titleCell('黑板在跑')).findByText('7', undefined, WAIT);
    await within(titleCell('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(within(titleCell('执行')).getByText('3')).toBeInTheDocument();
    expect(
      within(titleCell('会话')).getByText('今日已复核'),
    ).toBeInTheDocument();
    for (const [label, reason] of [
      ['agent 程序', '接口未提供 agent 程序版本。'],
      ['人侧发行', '接口未提供人侧发行号。'],
      ['副本大小', '接口未提供副本大小。'],
    ]) {
      const cell = titleCell(label);
      expect(within(cell).getByText('—')).toBeInTheDocument();
      expect(cell).toHaveAttribute('title', reason);
    }
    expect(titleCell('未关闭 BUG')).toHaveAttribute(
      'title',
      expect.stringContaining('不可得'),
    );
    expect(
      within(region('图纸标题栏')).getByText(/副本代次 243/),
    ).toBeInTheDocument();
  });

  it('本机健康读不到时只有取自健康的三格写原因，其余照常', async () => {
    renderCockpit({
      [HEALTH]: () =>
        new Response(JSON.stringify({ error: '本机未装配健康证据来源。' }), {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        }),
    });
    await within(titleCell('黑板在跑')).findByText('7', undefined, WAIT);
    for (const label of ['每轮同步', '同步时限', '会话'])
      await within(titleCell(label)).findByText(
        '本机健康读取失败：本机未装配健康证据来源。',
        undefined,
        WAIT,
      );
  });
});

describe('驾驶舱：黑板汇总的三种不顺', () => {
  it('黑板未启用（404）：只有画黑板的格子显示空态，其余格子照常、不报错', async () => {
    const { data } = renderCockpit({
      [OVERVIEW]: () => new Response(null, { status: 404 }),
    });
    const map = region('协作拓扑，今天');
    expect(
      await within(map).findByText('黑板未启用', undefined, WAIT),
    ).toBeInTheDocument();
    expect(screen.queryByTestId('topology')).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    // 待我动作只剩工作台待办，编号从 1 起、一律空心；总数就是它们的条数。
    const rows = within(region('待我动作')).getAllByRole('listitem');
    expect(rows).toHaveLength(data.workbench.inbox.length);
    for (const row of rows)
      expect(row.querySelector('.ui-circled')).toHaveAttribute(
        'data-filled',
        'false',
      );
    expect(within(hero()).getByText('05')).toBeInTheDocument();
    expect(heroNote()).toBeNull();
    // 项目明细与标题栏照常；黑板那两格写明原因。
    expect(within(region('项目明细')).getByRole('table')).toBeInTheDocument();
    for (const label of ['黑板在跑', '执行'])
      expect(titleCell(label)).toHaveAttribute('title', '黑板未启用');
    expect(within(titleCell('可读项目')).getByText('8')).toBeInTheDocument();
    // 时间线本就由黑板事件推出：同样是安静的空态，不是错误。
    expect(
      within(region('今天的时间线')).getByText('黑板未启用'),
    ).toBeInTheDocument();
  });

  it('读取失败：拓扑格报错并可重试，重试只重读黑板汇总', async () => {
    let failing = true;
    const { data, requests } = renderCockpit({
      [OVERVIEW]: () =>
        failing
          ? new Response(JSON.stringify({ error: '黑板许可已满。' }), {
              status: 429,
              headers: { 'Content-Type': 'application/json' },
            })
          : fixtureFetch(data.api)(OVERVIEW_URL),
    });
    const map = region('协作拓扑，今天');
    const alert = await within(map).findByRole('alert', undefined, WAIT);
    expect(alert).toHaveTextContent('黑板汇总读取失败');
    expect(alert).toHaveTextContent('黑板许可已满。');
    expect(screen.getAllByRole('alert')).toHaveLength(1);
    // 总数不可知就不给数；待我动作先列工作台待办并说明缺了黑板。
    expect(hero()).toHaveTextContent('—');
    expect(heroNote()).toHaveTextContent('黑板读取失败，总数暂不可得');
    expect(
      within(region('待我动作')).getByText(
        '黑板汇总读取失败，等你的黑板任务暂未列出。',
      ),
    ).toBeInTheDocument();
    const health = requests(HEALTH);
    failing = false;
    await userEvent
      .setup()
      .click(within(map).getByRole('button', { name: '重试黑板汇总' }));
    await screen.findByTestId('topology', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(2);
    expect(requests(HEALTH)).toBe(health);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('读取中：拓扑与时间线显示读取占位，巨号不给数', () => {
    renderCockpit({ [OVERVIEW]: () => new Promise<Response>(() => {}) });
    expect(
      within(region('协作拓扑，今天')).getByRole('status'),
    ).toHaveTextContent('正在读取黑板汇总…');
    expect(
      within(region('今天的时间线')).getByRole('status'),
    ).toHaveTextContent('正在读取黑板事件…');
    expect(heroNote()).toHaveTextContent('正在读取黑板，总数稍后给出');
    expect(hero()).toHaveTextContent('—');
  });

  it('一轮刷新结束后连带重读黑板汇总与本机健康', async () => {
    const { rerender, requests } = renderCockpit();
    await screen.findByTestId('topology', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(1);
    expect(requests(HEALTH)).toBe(1);
    rerender(true);
    expect(screen.getByText(/（刷新中）/)).toBeInTheDocument();
    rerender(false);
    await screen.findByTestId('topology', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(2);
    expect(requests(HEALTH)).toBe(2);
  });
});

describe('驾驶舱：今天的时间线', () => {
  it('泳道由黑板事件推出，你的审批是红块；瞬时的块也按最窄 0.6% 画出来', async () => {
    renderCockpit();
    const timeline = region('今天的时间线');
    const lanes = await within(timeline).findByRole(
      'img',
      { name: /^时间泳道 08:00–21:00/ },
      WAIT,
    );
    expect(lanes).toHaveAccessibleName(/你：审批 4 段/);
    const owner = within(lanes).getByText('你').nextElementSibling!;
    const widths = [...owner.querySelectorAll('rect')].map((rect) =>
      Number(rect.getAttribute('width')),
    );
    expect(widths).toHaveLength(4);
    for (const width of widths) expect(width).toBeCloseTo(0.6, 3);
    expect(
      [...owner.querySelectorAll('rect')].every((rect) =>
        rect.classList.contains('ui-lanes__block--owner'),
      ),
    ).toBe(true);
    const feed = within(timeline).getByRole('list', { name: '最近动态' });
    expect(within(feed).getAllByRole('listitem')).toHaveLength(3);
    expect(within(feed).getAllByRole('listitem')[0]).toHaveAttribute(
      'title',
      'runner.start（outcome）',
    );
  });
});

/** 路径以 prefix 开头的请求数。 */
function countSent(calls: readonly unknown[][], prefix: string) {
  return calls.filter(([input]) => String(input).startsWith(prefix)).length;
}

describe('驾驶舱：黑板汇总由 App 读一次交进来（T-162）', () => {
  /** 直接渲染驾驶舱、交一份现成的读取；返回按路径前缀数请求的函数。 */
  function renderWithRead(data: FixtureData, overview: OverviewRead) {
    const fetchMock = vi.fn(fixtureFetch(data.api));
    vi.stubGlobal('fetch', fetchMock);
    render(
      <CockpitView
        workbench={data.workbench}
        refreshing={false}
        overview={overview}
        onRefresh={vi.fn()}
        filters={null}
        actions={data.workbench.inbox}
        projects={data.workbench.projects}
        diagnostics={0}
        onOpen={vi.fn()}
        onEnterProject={vi.fn()}
        onViewProjects={vi.fn()}
        onShowDiagnostics={vi.fn()}
      />,
    );
    return (prefix: string) => countSent(fetchMock.mock.calls, prefix);
  }

  it('只用交进来的那一份，自己不另读黑板汇总；本机健康照旧自己读', async () => {
    const data = fixtureData();
    const reload = vi.fn();
    const requests = renderWithRead(data, {
      state: { kind: 'ready', overview: data.overview },
      reload,
    });
    // 交进来的就是读完的汇总：首屏即给数，不经过读取中。
    expect(within(hero()).getByText('8')).toHaveClass('sr-only');
    expect(within(region('待我动作')).getAllByRole('listitem')).toHaveLength(
      3 + data.workbench.inbox.length,
    );
    await within(titleCell('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(HEALTH)).toBe(1);
    expect(requests(OVERVIEW)).toBe(0);
    expect(reload).not.toHaveBeenCalled();
  });

  it('拓扑格的「重试」调交进来的 reload，自己不发请求', async () => {
    const reload = vi.fn();
    const requests = renderWithRead(fixtureData(), {
      state: { kind: 'error', message: '黑板许可已满。' },
      reload,
    });
    const alert = within(region('协作拓扑，今天')).getByRole('alert');
    expect(alert).toHaveTextContent('黑板许可已满。');
    expect(hero()).toHaveTextContent('—');
    await userEvent
      .setup()
      .click(within(alert).getByRole('button', { name: '重试黑板汇总' }));
    expect(reload).toHaveBeenCalledTimes(1);
    // 等本机健康读完再收尾，免得更新落在测试结束之后。
    await within(titleCell('每轮同步')).findByText('85 S', undefined, WAIT);
    expect(requests(OVERVIEW)).toBe(0);
  });
});
