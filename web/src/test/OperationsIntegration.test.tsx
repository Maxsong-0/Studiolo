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
import { declarationSource, operationsData } from './operationsFixtures';

it('从主导航进入项目运作，来源由现有原文弹窗按代次读取并归还焦点', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/project-operations')
      return jsonResponse(operationsData());
    if (url.pathname === '/api/document')
      return jsonResponse({
        path: declarationSource.path,
        title: '架构声明',
        content: '# 架构\n目录与模块职责的原始声明',
        navigation: emptyNavigation,
      });
    return defaultApiResponse(input);
  });
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: '项目运作' }));
  expect(
    screen.getByRole('heading', { level: 1, name: '项目运作' }),
  ).toBeInTheDocument();
  const source = await screen.findByRole('button', { name: '查看结构声明' });
  await user.click(source);
  const dialog = await screen.findByRole('dialog');
  expect(
    await screen.findByText('目录与模块职责的原始声明'),
  ).toBeInTheDocument();
  expect(dialog).toHaveTextContent(`${declarationSource.path} · 第 12 行`);
  const documentCall = fetchMock.mock.calls.find(([input]) =>
    String(input).startsWith('/api/document?'),
  );
  const query = new URL(String(documentCall?.[0]), 'http://localhost')
    .searchParams;
  expect(query.get('path')).toBe(declarationSource.path);
  expect(query.get('generation')).toBe('7');
  await user.click(screen.getByRole('button', { name: '关闭原文' }));
  await waitFor(() => expect(source).toHaveFocus());
});

it('选择项目和更新文档快照会请求对应结构，隐含父的来源选择不会残留', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const query = new URL(String(input), 'http://localhost').searchParams;
    const project = query.get('project')!;
    return jsonResponse(
      operationsData({
        project,
        generation: Number(query.get('generation')),
        directories: [
          {
            path:
              project === fixture.projects[0].root ? '初始目录' : '新项目目录',
            label: '此项目的职责',
            source: declarationSource,
          },
        ],
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
  await screen.findByRole('button', { name: '选择目录 初始目录' });
  await user.selectOptions(
    screen.getByRole('combobox', { name: '项目' }),
    fixture.projects[1].root,
  );
  await screen.findByRole('button', { name: '选择目录 新项目目录' });
  expect(
    screen.queryByRole('button', { name: '选择目录 初始目录' }),
  ).not.toBeInTheDocument();
  rerender(<ProjectOperations {...props} generation={8} />);
  await screen.findByText('仓库目录按本次读取独立观测；文档声明绑定快照 8。');
  expect(screen.getByRole('combobox')).toHaveValue(fixture.projects[1].root);
  const request = new URL(
    String(fetchMock.mock.calls.at(-1)?.[0]),
    'http://localhost',
  );
  expect(request.searchParams.get('project')).toBe(fixture.projects[1].root);
  expect(request.searchParams.get('generation')).toBe('8');
});
