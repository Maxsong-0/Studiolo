import {
  act,
  configure,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { TASK } from '../../../dev/fixtures/ids';
import { CollabView } from '../../views/CollabView';
import { parseViewLocation } from '../../viewLocation';

/*
 * 拓扑整页有上百个 SVG 与清单节点：角色查询默认要对每个节点算一遍 getComputedStyle 判可见性，
 * 机器忙时一次查询就要一两秒。jsdom 里本来不加载样式表，可见性判断没有意义，这里关掉它，
 * 并放宽异步等待，免得断言本身没错却超时。只影响本文件（每个测试文件各自一份模块）。
 */
configure({ defaultHidden: true, asyncUtilTimeout: 20_000 });

/** 假数据的「此刻」：今天 20:31（本地），时间带正好是 08:00–20:00。 */
const NOW = new Date(2026, 8, 22, 20, 31).getTime();
const studiolo = '项目/Studiolo';
const brief = '项目/Brief 简报站';
const projects = [
  { root: studiolo, name: 'Studiolo' },
  { root: brief, name: 'Brief 简报站' },
];

function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

/** 用开发假数据的应答器当后端；override 返回 Response 时改用它。 */
function serve(override?: (url: string) => Response | undefined) {
  const api = createFixtureApi({ now: () => NOW });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const custom = override?.(url);
    if (custom) return custom;
    const outcome = api.handle({
      method: init?.method ?? 'GET',
      url,
      headers: {},
    });
    return json(outcome.body ?? null, outcome.status);
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

async function mount(hash: string) {
  window.location.hash = hash;
  await act(async () => {
    render(<CollabView projects={projects} />);
  });
}

const location = () => parseViewLocation(window.location.hash);
const outline = () => screen.getByRole('list', { name: /按执行者列出$/ });
const listed = () =>
  within(outline())
    .getAllByRole('button')
    .map((button) => button.textContent);
const boardReads = (fetch: ReturnType<typeof serve>) =>
  fetch.mock.calls.filter(([url]) => String(url).startsWith('/api/v1/board?'));

describe('协作视图', { timeout: 60_000 }, () => {
  it('切子标签写 panel，项目与条目原样留着；拓扑记作缺省面板', async () => {
    serve();
    await mount(
      `#view=collab&project=${encodeURIComponent(studiolo)}&item=${TASK.T130}`,
    );
    await screen.findByRole('img', { name: '协作拓扑：Studiolo' });
    expect(screen.getByRole('tab', { name: '拓扑' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    const detail = screen.getByRole('complementary', { name: '任务详情' });
    expect(
      within(detail).getByRole('heading', { name: 'T-130 前端全面重设计' }),
    ).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: '任务与回执' }));
    });
    expect(location()).toEqual({
      view: 'collab',
      project: studiolo,
      panel: 'board',
      item: TASK.T130,
    });
    expect(
      await screen.findByRole('article', { name: 'T-130 前端全面重设计' }),
    ).toHaveAttribute('aria-current', 'true');

    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: '时间线' }));
    });
    expect(location().panel).toBe('timeline');
    expect(
      screen.getByRole('region', { name: '项目时间回放' }),
    ).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: '拓扑' }));
    });
    expect(location()).toEqual({
      view: 'collab',
      project: studiolo,
      panel: null,
      item: TASK.T130,
    });
  });

  it('右上角汇总与两个开关：只看冲突留下重叠两端，关掉显示休眠藏起 T-046', async () => {
    serve();
    await mount('#view=collab');
    await screen.findByRole('img', { name: '协作拓扑' });
    expect(screen.getByText('6 条在跑')).not.toHaveClass('is-signal');
    expect(screen.getByText('1 处写域重叠')).toHaveClass('is-signal');
    expect(screen.getByText('3 处等你')).toHaveClass('is-signal');
    expect(listed()).toHaveLength(7);
    expect(document.querySelectorAll('.topology__path--overlap')).toHaveLength(
      2,
    );
    expect(document.querySelectorAll('.topology__bracket')).toHaveLength(1);

    const conflicts = screen.getByRole('checkbox', { name: '只看冲突' });
    expect(conflicts).not.toBeChecked();
    await act(async () => {
      fireEvent.click(conflicts);
    });
    expect(listed()).toEqual(['T-092 人侧 M4 L3 部署', 'T-130 前端全面重设计']);
    expect(document.querySelectorAll('.topology__task')).toHaveLength(2);
    await act(async () => {
      fireEvent.click(conflicts);
    });
    expect(listed()).toHaveLength(7);

    const dormant = screen.getByRole('checkbox', { name: '显示休眠' });
    expect(dormant).toBeChecked();
    expect(listed()).toContain('T-046 M2/M3 整体收尾');
    await act(async () => {
      fireEvent.click(dormant);
    });
    expect(listed()).toHaveLength(6);
    expect(listed()).not.toContain('T-046 M2/M3 整体收尾');
    // 开关只影响画什么，汇总数照旧
    expect(screen.getByText('1 处写域重叠')).toBeInTheDocument();
  });

  it('选中任务写进 item，右侧详情列出认领、写域与重叠；没有回执时不读黑板明细', async () => {
    const fetch = serve();
    await mount('#view=collab');
    await screen.findByRole('img', { name: '协作拓扑' });
    const detail = () =>
      screen.getByRole('complementary', { name: '任务详情' });
    expect(within(detail()).getByText('未选中任务')).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(
        within(outline()).getByRole('button', {
          name: 'T-130 前端全面重设计',
        }),
      );
    });
    expect(location().item).toBe(TASK.T130);
    expect(
      within(outline()).getByRole('button', { name: 'T-130 前端全面重设计' }),
    ).toHaveAttribute('aria-pressed', 'true');
    expect(document.querySelector(`[data-task-id="${TASK.T130}"]`)).toHaveClass(
      'topology__task--selected',
    );
    expect(
      within(detail()).getByRole('heading', { name: 'T-130 前端全面重设计' }),
    ).toBeInTheDocument();
    // 五个根实例的 id 前 4 位有撞的：标签统一加长到 5 位
    expect(detail()).toHaveTextContent('Claude · 01M34 / 01M34E0N');
    expect(detail()).toHaveTextContent('进行中');
    expect(detail()).toHaveTextContent(
      'web/src/policyApi.tsT-092 人侧 M4 L3 部署 · 已审阅通过；落在本任务的 web 里',
    );
    expect(detail()).toHaveTextContent('任务尚未结束，还没有回执。');
    expect(boardReads(fetch)).toHaveLength(0);

    // 点 SVG 里的任务框同样选中
    await act(async () => {
      fireEvent.click(document.querySelector(`[data-task-id="${TASK.T109}"]`)!);
    });
    expect(location().item).toBe(TASK.T109);
    await act(async () => {
      fireEvent.click(
        within(detail()).getByRole('button', { name: '在任务与回执中查看' }),
      );
    });
    expect(location()).toEqual({
      view: 'collab',
      project: studiolo,
      panel: 'board',
      item: TASK.T109,
    });
  });

  it('选中有回执的条目时按需读一次该项目的黑板明细，填出回执五字段', async () => {
    const fetch = serve();
    await mount(`#view=collab&item=${TASK.B021}`);
    const receipt = await screen.findByRole('region', { name: '回执' });
    await within(receipt).findByText('2 通过');
    const field = (label: string) =>
      within(receipt).getByText(label, { selector: 'dt' }).nextElementSibling
        ?.textContent;
    expect(field('状态')).toBe('已完成');
    expect(field('改动文件')).toBe('1 个');
    expect(field('冲突')).toBe('0 处');
    expect(field('验收自评')).toBe('2 通过');
    expect(field('剩余事项')).toBe('无');
    expect(boardReads(fetch).map(([url]) => String(url))).toEqual([
      `/api/v1/board?project=${encodeURIComponent(brief)}`,
    ]);
  });

  it('回执明细读不到时五字段都是「—」并写明原因', async () => {
    serve((url) =>
      url.startsWith('/api/v1/board?')
        ? json({ error: '黑板许可繁忙' }, 429)
        : undefined,
    );
    await mount(`#view=collab&item=${TASK.B021}`);
    const receipt = await screen.findByRole('region', { name: '回执' });
    await within(receipt).findByText('黑板许可繁忙');
    const values = within(receipt)
      .getAllByRole('definition')
      .map((node) => node.textContent);
    // 状态这一项汇总接口已经给了结论，其余四项要明细
    expect(values).toEqual(['已完成', '—', '—', '—', '—']);
  });

  it('黑板未启用（汇总 404）时拓扑格显示空态与原因，汇总数写「—」', async () => {
    serve((url) =>
      url.startsWith('/api/v1/board/overview')
        ? json({ error: '本机未装配黑板。' }, 404)
        : undefined,
    );
    await mount('#view=collab');
    expect(await screen.findByText('黑板未启用')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
    expect(screen.getByText('— 条在跑')).toBeInTheDocument();
    expect(screen.getByText('— 处等你')).toBeInTheDocument();
  });

  it('汇总读取失败显示错误与重试，重试成功后画出拓扑', async () => {
    let failing = true;
    serve((url) =>
      failing && url.startsWith('/api/v1/board/overview')
        ? json({ error: '黑板许可繁忙' }, 429)
        : undefined,
    );
    await mount('#view=collab');
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('实时状态不可确认');
    expect(alert).toHaveTextContent('黑板许可繁忙');
    failing = false;
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '重新读取' }));
    });
    expect(
      await screen.findByRole('img', { name: '协作拓扑' }),
    ).toBeInTheDocument();
  });

  it('任务与回执、时间线、执行要先选项目：未选时给空态与项目选择，不发请求', async () => {
    const fetch = serve();
    await mount('#view=collab&panel=board');
    expect(screen.getByText('先选一个项目')).toBeInTheDocument();
    expect(fetch).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.change(screen.getByRole('combobox', { name: '项目' }), {
        target: { value: studiolo },
      });
    });
    expect(location()).toEqual({
      view: 'collab',
      project: studiolo,
      panel: 'board',
      item: null,
    });
    expect(
      await screen.findByRole('region', { name: '协作黑板' }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole('combobox', { name: '项目' })).toHaveLength(1);
  });

  it('拓扑的项目筛选写进 project 并清掉条目；「全部项目」写回 null', async () => {
    serve();
    await mount(`#view=collab&item=${TASK.T130}`);
    await screen.findByRole('img', { name: '协作拓扑' });
    const filter = screen.getByRole('combobox', { name: '项目' });
    expect(filter).toHaveValue('');
    await act(async () => {
      fireEvent.change(filter, { target: { value: brief } });
    });
    expect(location()).toEqual({
      view: 'collab',
      project: brief,
      panel: null,
      item: null,
    });
    // 简报站只有一条已结束的任务：拓扑没有要画的，时间线照样有今天的事件
    expect(screen.getByText('黑板上没有未结束的任务')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /^时间泳道/ })).toBeInTheDocument();
    await act(async () => {
      fireEvent.change(screen.getByRole('combobox', { name: '项目' }), {
        target: { value: '' },
      });
    });
    expect(location().project).toBeNull();
  });
});
