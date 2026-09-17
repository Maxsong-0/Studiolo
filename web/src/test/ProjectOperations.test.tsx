import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { ProjectOperations } from '../components/ProjectOperations';
import { fixture, jsonResponse } from './fixtures';
import {
  declarationSource,
  directorySource,
  operationsData,
} from './operationsFixtures';

const fetchMock = vi.fn<typeof fetch>();
beforeEach(() => {
  fetchMock.mockReset().mockResolvedValue(jsonResponse(operationsData()));
  vi.stubGlobal('fetch', fetchMock);
});

function mount() {
  const onOpen = vi.fn();
  const onRefresh = vi.fn();
  render(
    <ProjectOperations
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  return { onOpen, onRefresh };
}

it('项目请求绑定根路径和文档代次；选择目录与声明入口均定位真实来源', async () => {
  const user = userEvent.setup();
  const { onOpen } = mount();
  const graph = await screen.findByRole('region', { name: '目录与模块职责图' });
  const request = new URL(
    String(fetchMock.mock.calls[0][0]),
    'http://localhost',
  );
  expect(request.pathname).toBe('/api/project-operations');
  expect(request.searchParams.get('project')).toBe(fixture.projects[0].root);
  expect(request.searchParams.get('generation')).toBe('7');
  expect(
    within(graph).getByRole('button', { name: '选择目录 crates/core' }),
  ).toHaveAttribute('aria-pressed', 'true');
  await user.click(screen.getByRole('button', { name: '查看职责声明原文' }));
  expect(onOpen).toHaveBeenLastCalledWith(directorySource);
  await user.click(within(graph).getByRole('button', { name: '选择目录 web' }));
  expect(
    screen.getByRole('complementary', { name: '所选目录详情' }),
  ).toHaveTextContent('呈现只读工作台');
  await user.click(screen.getByRole('button', { name: '查看职责声明原文' }));
  expect(onOpen).toHaveBeenLastCalledWith({ ...directorySource, line: 17 });
  await user.click(screen.getByRole('button', { name: '查看结构声明' }));
  expect(onOpen).toHaveBeenLastCalledWith(declarationSource);
  await user.click(
    screen.getByRole('button', { name: 'crates/core，查看目录声明来源' }),
  );
  expect(onOpen).toHaveBeenLastCalledWith(directorySource);
});

it('图中隐含父目录不冒充职责声明，按钮可由键盘选中', async () => {
  const user = userEvent.setup();
  mount();
  const node = await screen.findByRole('button', { name: '选择目录 crates' });
  node.focus();
  await user.keyboard('{Enter}');
  expect(node).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByRole('complementary')).toHaveTextContent(
    '此节点由目录路径推导，没有人工声明的模块职责。',
  );
  expect(
    screen.queryByRole('button', { name: '查看职责声明原文' }),
  ).not.toBeInTheDocument();
});

it('目录职责和诊断中的HTML按纯文本呈现', async () => {
  const hostile =
    '<img src=x onerror="alert(1)"><svg onload="alert(2)">中文模块';
  fetchMock.mockResolvedValue(
    jsonResponse(
      operationsData({
        directories: [{ path: 'web', label: hostile, source: directorySource }],
        diagnostics: ['<script>alert(3)</script>'],
      }),
    ),
  );
  mount();
  await screen.findByRole('region', { name: '目录与模块职责图' });
  expect(screen.getAllByText(hostile)).toHaveLength(2);
  expect(screen.getByText('<script>alert(3)</script>')).toBeInTheDocument();
  expect(document.querySelector('img, script, svg[onload]')).toBeNull();
});

it('双向漂移区分缺失与未声明，并限定父域和独立观测时间', async () => {
  mount();
  await screen.findByRole('list', { name: '目录比对结果' });
  expect(screen.getByText('已声明 · 目录存在')).toBeInTheDocument();
  expect(screen.getByText('已声明 · 目录缺失')).toBeInTheDocument();
  expect(screen.getByText('目录存在 · 未声明')).toBeInTheDocument();
  expect(screen.getByText(/此结果不代表全仓库检查通过/)).toBeInTheDocument();
  expect(screen.getByText('比较父目录')).toBeInTheDocument();
  expect(
    screen.getByText('仓库目录按本次读取独立观测；文档声明绑定快照 7。'),
  ).toBeInTheDocument();
  expect(document.querySelector('time')).toHaveAttribute(
    'dateTime',
    new Date(1789140000 * 1000).toISOString(),
  );
});

it.each(['unbound', 'unavailable'] as const)(
  '仓库%s不把目录状态显示成已核验',
  async (status) => {
    fetchMock.mockResolvedValue(
      jsonResponse(
        operationsData({ repository: { status, observed_at: null } }),
      ),
    );
    mount();
    const drift = await screen.findByRole('list', { name: '目录比对结果' });
    expect(within(drift).getAllByText('未核验')).toHaveLength(3);
    expect(
      within(drift).queryByText('已声明 · 目录存在'),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/暂无可信观测时间/)).toBeInTheDocument();
  },
);

it.each(['unavailable', 'invalid'] as const)(
  '结构声明%s时保留解释与原文入口',
  async (declaration_status) => {
    fetchMock.mockResolvedValue(
      jsonResponse(
        operationsData({
          declaration_status,
          directories: [],
          drift: [],
          diagnostics: ['第12行：目录声明缺少标签'],
        }),
      ),
    );
    const { onOpen } = mount();
    expect(
      await screen.findByText(/当前无法生成目录与模块职责图/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('region', { name: '目录与模块职责图' }),
    ).not.toBeInTheDocument();
    expect(screen.getByText('第12行：目录声明缺少标签')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '查看结构声明' }));
    expect(onOpen).toHaveBeenCalledWith(declarationSource);
  },
);

it.each([409, 429, 503])('%s错误提供刷新或重试且不展示旧图', async (status) => {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({ error: '读取暂不可用' }, status),
  );
  const { onRefresh } = mount();
  expect(await screen.findByRole('alert')).toHaveTextContent('读取暂不可用');
  expect(
    screen.queryByRole('region', { name: '目录与模块职责图' }),
  ).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole('button', {
      name: status === 409 ? '刷新工作台' : '重试',
    }),
  );
  if (status === 409) expect(onRefresh).toHaveBeenCalledOnce();
  else
    expect(
      await screen.findByRole('region', { name: '目录与模块职责图' }),
    ).toBeInTheDocument();
});

it('损坏响应给出可恢复错误；重新核验重新读取实际目录', async () => {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({ ...operationsData(), directories: [null] }),
  );
  mount();
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '项目运作数据格式不完整',
  );
  await userEvent.click(screen.getByRole('button', { name: '重试' }));
  await screen.findByRole('region', { name: '目录与模块职责图' });
  fetchMock.mockResolvedValueOnce(
    jsonResponse(
      operationsData({
        drift: [{ path: 'web', status: 'present', source: directorySource }],
      }),
    ),
  );
  await userEvent.click(screen.getByRole('button', { name: '重新核验目录' }));
  const drift = await screen.findByRole('list', { name: '目录比对结果' });
  expect(within(drift).getAllByRole('listitem')).toHaveLength(1);
  expect(screen.queryByText('已声明 · 目录缺失')).not.toBeInTheDocument();
});
