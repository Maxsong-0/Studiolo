import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import {
  defaultApiResponse,
  documents,
  emptyNavigation,
  fixture,
  jsonResponse,
} from './fixtures';

const fetchMock = vi.fn<typeof fetch>();

/** 工作台请求的先后。状态胶囊在外壳里另读本机健康，那条请求不在这里数。 */
function workbenchRequests() {
  return fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((path) => path.startsWith('/api/workbench'));
}

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => defaultApiResponse(input));
  vi.stubGlobal('fetch', fetchMock);
});

describe('只读工作台', () => {
  it('从 API 读取真实快照，筛选项目和生命周期，并保留未知值', async () => {
    const user = userEvent.setup();
    render(<App />);
    // 数据来源标签收进了顶栏状态浮层：先打开胶囊再核对。
    await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
    expect(await screen.findByText('测试只读库')).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][0]).toBe('/api/workbench');
    // 项目明细全表与筛选在「项目」视图（T-130 规范第 4、9 节）；驾驶舱只摆前五行摘要。
    await user.click(screen.getByRole('button', { name: '项目' }));
    const unknownRow = screen.getByRole('row', { name: /灵感盒/ });
    expect(within(unknownRow).getAllByText('未知')).toHaveLength(5);
    expect(within(unknownRow).queryByText('0')).not.toBeInTheDocument();
    await user.type(
      screen.getByRole('searchbox', { name: '筛选项目' }),
      '星舟',
    );
    expect(
      screen.queryByRole('row', { name: /资料夹/ }),
    ).not.toBeInTheDocument();
    await user.clear(screen.getByRole('searchbox'));
    await user.selectOptions(
      screen.getByRole('combobox', { name: '生命周期' }),
      '静态资料',
    );
    expect(screen.getByRole('row', { name: /资料夹/ })).toBeInTheDocument();
    expect(screen.queryByRole('row', { name: /星舟/ })).not.toBeInTheDocument();
    // 驾驶舱是全局总览、没有筛选栏：项目视图的筛选不截断待我动作（规范第 5、6 节）。
    await user.click(screen.getByRole('button', { name: '驾驶舱' }));
    expect(
      screen.getByRole('button', { name: '验收只读工作台，查看原文' }),
    ).toBeInTheDocument();
  });

  it('来源携带 generation、定位目标行，原文 HTML 只作为文本，关闭后还原焦点', async () => {
    fetchMock.mockImplementation(async (input) =>
      String(input).startsWith('/api/document')
        ? jsonResponse({
            path: documents[0].path,
            title: '星舟总览',
            navigation: emptyNavigation,
            content:
              '# 总览\n<img src=x onerror="alert(1)">\n<script>alert(2)</script>\n最后一行',
          })
        : defaultApiResponse(input),
    );
    const user = userEvent.setup();
    render(<App />);
    // 当前版本列在「项目」视图的明细全表里；驾驶舱的项目明细只有序号、项目、状态与备注。
    await screen.findByRole('table');
    await user.click(screen.getByRole('button', { name: '项目' }));
    const sourceButton = await screen.findByRole('button', {
      name: '星舟 当前版本：M1，查看来源',
    });
    await user.click(sourceButton);
    const dialog = screen.getByRole('dialog');
    expect(
      await within(dialog).findByText('<script>alert(2)</script>'),
    ).toHaveClass('line-content');
    expect(
      within(dialog).getByText('<img src=x onerror="alert(1)">'),
    ).toBeInTheDocument();
    expect(dialog.querySelector('script, img')).toBeNull();
    expect(dialog.querySelector('[aria-current="location"]')).toHaveTextContent(
      '<script>alert(2)</script>',
    );
    expect(
      within(dialog).getByText(`${documents[0].path} · 第 3 行`),
    ).toBeInTheDocument();
    const request = fetchMock.mock.calls.find(([input]) =>
      String(input).startsWith('/api/document'),
    )!;
    const url = new URL(String(request[0]), 'http://localhost');
    expect(url.searchParams.get('path')).toBe(documents[0].path);
    expect(url.searchParams.get('generation')).toBe('7');
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(sourceButton).toHaveFocus();
  });

  it('三处导航和查看全部工作，文档打开只读原文', async () => {
    fetchMock.mockImplementation(async (input) =>
      String(input).startsWith('/api/document')
        ? jsonResponse({
            path: documents[0].path,
            title: '星舟总览',
            content: '# 星舟',
            navigation: emptyNavigation,
          })
        : defaultApiResponse(input),
    );
    const user = userEvent.setup();
    render(<App />);
    // 驾驶舱项目明细只列前五行，入口写明总数（规范第 5 节，承接 T-117）。
    await user.click(
      await screen.findByRole('button', { name: '全部 3 个项目，打开项目' }),
    );
    expect(
      screen.getByRole('heading', { level: 1, name: '项目' }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '文档' }));
    expect(
      screen.getByRole('heading', { level: 1, name: '文档' }),
    ).toBeInTheDocument();
    await user.click(
      await screen.findByRole('button', { name: '星舟总览，查看原文' }),
    );
    expect(await screen.findByText('# 星舟')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '关闭原文' }));
    await user.click(screen.getByRole('button', { name: '驾驶舱' }));
    expect(screen.getByRole('table')).toBeInTheDocument();
  });

  it('失败可重试，刷新失败继续显示旧快照但明确提示', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ error: '副本不可用' }, 422));
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('副本不可用');
    await user.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(workbenchRequests()).toEqual([
      '/api/workbench',
      '/api/workbench?refresh=true',
    ]);
    fetchMock.mockResolvedValueOnce(jsonResponse({ error: '读取中断' }, 422));
    // 刷新按钮挪进了顶栏状态浮层。
    await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
    await user.click(screen.getByRole('button', { name: '刷新快照' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '刷新失败，仍显示上次快照。',
    );
    expect(screen.getByRole('table')).toBeInTheDocument();
  });

  it('409 说明快照过期并可刷新，不用新内容冒充旧来源', async () => {
    fetchMock.mockImplementation(async (input) =>
      String(input).startsWith('/api/document')
        ? jsonResponse({ error: 'snapshot changed' }, 409)
        : defaultApiResponse(input),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      await screen.findByRole('button', { name: '验收只读工作台，查看原文' }),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '快照已更新，请刷新工作台',
    );
    await user.click(screen.getByRole('button', { name: '刷新工作台' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
    );
    expect(
      fetchMock.mock.calls.some(
        ([input]) => input === '/api/workbench?refresh=true',
      ),
    ).toBe(true);
  });

  it('空快照、筛选空态和空文档有明确反馈', async () => {
    fetchMock.mockImplementation(async (input) =>
      defaultApiResponse(
        input,
        {
          ...fixture,
          projects: [],
          inbox: [],
          counts: {
            ...fixture.counts,
            documents: { value: 0, unavailable: [] },
            projects: { value: 0, unavailable: [] },
            inbox: { value: 0, unavailable: [] },
          },
        },
        [],
      ),
    );
    const user = userEvent.setup();
    render(<App />);
    // 驾驶舱：项目明细与待我动作各自说明为空（黑板汇总回 404 后待我动作才算读完）。
    expect(
      await screen.findByText('当前快照没有可读项目。'),
    ).toBeInTheDocument();
    expect(await screen.findByText('当前没有待我动作。')).toBeInTheDocument();
    // 带筛选的明细全表在「项目」视图，空态文案仍提示调整筛选。
    await user.click(screen.getByRole('button', { name: '项目' }));
    expect(
      screen.getByText('没有匹配的项目。可以调整筛选，或检查只读副本。'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '文档' }));
    expect(
      await screen.findByText('当前快照没有可读取的文档。'),
    ).toBeInTheDocument();
  });

  it('异常响应形状显示可恢复错误，而不是让界面崩溃', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ projects: [null] }));
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '工作台数据格式不完整',
    );
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
  });

  it('大文档只显示目标附近原文，并说明范围', async () => {
    const content = Array.from(
      { length: 2000 },
      (_, index) => `原始第${index + 1}行 ${'原文'.repeat(30)}`,
    ).join('\n');
    fetchMock.mockImplementation(async (input) =>
      String(input).startsWith('/api/document')
        ? jsonResponse({
            path: documents[0].path,
            title: '大文档',
            navigation: emptyNavigation,
            content,
          })
        : defaultApiResponse(input),
    );
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      await screen.findByRole('button', { name: '验收只读工作台，查看原文' }),
    );
    expect(
      await screen.findByText('文档较大，仅显示目标附近原文（第 1–103 行）'),
    ).toBeInTheDocument();
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getAllByRole('listitem')).toHaveLength(103);
    expect(dialog.querySelector('[aria-current="location"]')).toHaveTextContent(
      '原始第3行',
    );
    expect(within(dialog).queryByText(/^原始第104行 /)).not.toBeInTheDocument();
  });

  it('数据提示不再挂在页面底部：驾驶舱只报条数，条目在状态浮层里逐条列出', async () => {
    fetchMock.mockImplementation(async (input) =>
      defaultApiResponse(input, {
        ...fixture,
        diagnostics: ['副本里有两条同名笔记'],
        projects: fixture.projects.map((project) =>
          project.id === 'inbox'
            ? { ...project, diagnostics: ['缺少 INDEX.md'] }
            : project,
        ),
      }),
    );
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole('table');
    const main = screen.getByRole('main');
    expect(
      within(main).getByText('当前快照有 2 条数据提示：缺失信息保持未知。'),
    ).toBeInTheDocument();
    expect(
      within(main).queryByText('副本里有两条同名笔记'),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('region', { name: '内容新鲜度' }),
    ).not.toBeInTheDocument();
    for (const view of ['项目', '文档', '协作', '策略', '驾驶舱']) {
      await user.click(screen.getByRole('button', { name: view }));
      expect(
        within(screen.getByRole('main')).queryByText('副本里有两条同名笔记'),
      ).not.toBeInTheDocument();
    }
    await user.click(screen.getByRole('button', { name: '查看数据提示' }));
    const capsule = screen.getByRole('button', { name: /^工作台状态/ });
    expect(capsule).toHaveAttribute('aria-expanded', 'true');
    const popover = document.getElementById(
      capsule.getAttribute('aria-controls')!,
    )!;
    expect(popover).toHaveFocus();
    expect(within(popover).getByText('数据提示（2）')).toBeInTheDocument();
    expect(
      within(popover).getByText('副本里有两条同名笔记'),
    ).toBeInTheDocument();
    expect(
      within(popover).getByText('灵感盒：缺少 INDEX.md'),
    ).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(capsule).toHaveAttribute('aria-expanded', 'false');
    expect(capsule).toHaveFocus();
  });
});
