import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { App } from '../App';
import { ProjectOperations } from '../components/ProjectOperations';
import {
  defaultApiResponse,
  emptyNavigation,
  fixture,
  jsonResponse,
} from './fixtures';
import { architectureData, architectureSource } from './architectureFixtures';
import { operationsData } from './operationsFixtures';

it('节点来源通过现有原文弹窗携带正确文档代次并恢复焦点', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/project-operations')
      return jsonResponse(operationsData({ architecture: architectureData() }));
    if (url.pathname === '/api/document')
      return jsonResponse({
        path: architectureSource.path,
        title: '项目运作',
        navigation: emptyNavigation,
        content: '# 架构\n\n## 节点\n\n表头\n分隔\n检索接口原始职责',
      });
    return defaultApiResponse(input);
  });
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('table');
  // 项目运作并进了「项目」：从明细表点项目名进入它的运作。
  await user.click(screen.getByRole('button', { name: '项目' }));
  await user.click(screen.getByRole('button', { name: '星舟，进入项目运作' }));
  const source = await screen.findByRole('button', {
    name: '查看节点声明原文',
  });
  await user.click(source);
  expect(await screen.findByText('检索接口原始职责')).toBeInTheDocument();
  expect(screen.getByRole('dialog')).toHaveTextContent(
    `${architectureSource.path} · 第 7 行`,
  );
  const request = fetchMock.mock.calls.find(([input]) =>
    String(input).startsWith('/api/document?'),
  );
  const query = new URL(String(request?.[0]), 'http://localhost').searchParams;
  expect(query.get('path')).toBe(architectureSource.path);
  expect(query.get('generation')).toBe('7');
  await user.click(screen.getByRole('button', { name: '关闭原文' }));
  await waitFor(() => expect(source).toHaveFocus());
});

it('文档代次更新后旧设计步骤卸载，重新读取后回到新声明初始状态', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const query = new URL(String(input), 'http://localhost').searchParams;
    return jsonResponse(
      operationsData({
        generation: Number(query.get('generation')),
        architecture: architectureData(),
      }),
    );
  });
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  const props = {
    projects: fixture.projects,
    onOpen: vi.fn(),
    onRefresh: vi.fn(),
  };
  const { rerender } = render(<ProjectOperations {...props} generation={7} />);
  await user.click(await screen.findByRole('button', { name: '设计路径' }));
  await user.click(screen.getByRole('button', { name: '后一步' }));
  expect(screen.getByText('设计顺序 · 第 2 / 3 步')).toBeVisible();
  rerender(<ProjectOperations {...props} generation={8} />);
  expect(
    screen.queryByRole('list', { name: '设计步骤' }),
  ).not.toBeInTheDocument();
  await screen.findByRole('group', { name: '架构节点' });
  await user.click(screen.getByRole('button', { name: '设计路径' }));
  expect(screen.getByText('设计顺序 · 第 1 / 3 步')).toBeVisible();
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

it('存在但损坏的架构响应呈现可重试错误，不能悄悄变成无架构声明', async () => {
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ ...operationsData(), architecture: null }),
      ),
  );
  render(
    <ProjectOperations
      projects={fixture.projects}
      generation={7}
      onOpen={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '架构关系数据格式不完整',
  );
  expect(screen.queryByText('暂无可用的架构关系声明')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
});
