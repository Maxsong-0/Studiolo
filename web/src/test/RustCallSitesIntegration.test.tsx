import { act, render, screen, waitFor } from '@testing-library/react';
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
import { rustData } from './rustCallFixtures';

it('源码选择不读取Vault，目录关联声明仍按文档代次打开并归还焦点', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/project-operations')
      return jsonResponse(
        operationsData({
          rust_call_sites: rustData(),
          architecture: architectureData(),
        }),
      );
    if (url.pathname === '/api/document')
      return jsonResponse({
        path: architectureSource.path,
        title: '项目运作',
        navigation: emptyNavigation,
        content: '# 架构声明\n\n\n\n\n\n\n目录包含职责原文',
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
  await user.click(
    await screen.findByRole('button', {
      name: '选择 Rust 文件 crates/core/internal/lib.rs',
    }),
  );
  expect(
    fetchMock.mock.calls.some(([input]) =>
      String(input).startsWith('/api/document?'),
    ),
  ).toBe(false);
  const source = screen.getByRole('button', {
    name: '资料索引，查看所在目录的 Vault 声明',
  });
  await user.click(source);
  expect(await screen.findByText('目录包含职责原文')).toBeInTheDocument();
  const request = fetchMock.mock.calls.find(([input]) =>
    String(input).startsWith('/api/document?'),
  );
  const query = new URL(String(request?.[0]), 'http://localhost').searchParams;
  expect(query.get('path')).toBe(architectureSource.path);
  expect(query.get('generation')).toBe('7');
  await user.click(screen.getByRole('button', { name: '关闭原文' }));
  await waitFor(() => expect(source).toHaveFocus());
});

it('损坏的present响应报可重试错误，不能吞掉成尚无功能', async () => {
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockImplementation(async () =>
        jsonResponse({ ...operationsData(), rust_call_sites: null }),
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
    'Rust 调用位置数据格式、来源或覆盖范围不完整',
  );
  expect(screen.queryByText('暂无 Rust 调用位置数据')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
});

it('顶栏同代刷新保留所选项目，卸载旧证据并清理文件与种类选择', async () => {
  let finish!: (response: Response) => void;
  let workbenchReads = 0;
  let rustReads = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>().mockImplementation(async (input) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname === '/api/workbench' && ++workbenchReads === 2)
        return new Promise((resolve) => {
          finish = resolve;
        });
      if (url.pathname === '/api/project-operations') {
        rustReads += 1;
        return jsonResponse(
          operationsData({
            project: url.searchParams.get('project')!,
            rust_call_sites: rustData(),
          }),
        );
      }
      return defaultApiResponse(input);
    }),
  );
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('table');
  // 项目运作并进了「项目」：从明细表点项目名进入它的运作。
  await user.click(screen.getByRole('button', { name: '项目' }));
  await user.click(screen.getByRole('button', { name: '星舟，进入项目运作' }));
  await screen.findByRole('group', { name: 'Rust 白名单文件' });
  await user.selectOptions(
    screen.getByRole('combobox', { name: '项目' }),
    fixture.projects[1].root,
  );
  await user.click(
    await screen.findByRole('button', { name: '选择 Rust 文件 src/empty.rs' }),
  );
  await user.selectOptions(
    screen.getByRole('combobox', { name: '调用语法种类' }),
    'macro',
  );
  // 刷新按钮挪进了顶栏状态浮层。
  await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
  await user.click(screen.getByRole('button', { name: '刷新快照' }));
  expect(
    screen.queryByRole('group', { name: 'Rust 白名单文件' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('combobox', { name: '项目' })).toHaveValue(
    fixture.projects[1].root,
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '项目运作证据将在刷新后重新读取',
  );
  await act(async () => finish(jsonResponse(fixture)));
  expect(
    await screen.findByRole('button', {
      name: '选择 Rust 文件 crates/core/internal/lib.rs',
    }),
  ).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByRole('combobox', { name: '调用语法种类' })).toHaveValue(
    'all',
  );
  expect(screen.getByRole('combobox', { name: '项目' })).toHaveValue(
    fixture.projects[1].root,
  );
  expect(rustReads).toBe(3);
});
