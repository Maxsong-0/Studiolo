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
import { cargoData } from './cargoFixtures';
import { operationsData } from './operationsFixtures';

it('Cargo选择不请求Vault原文，只有精确关联的Vault按钮使用既有来源弹窗', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/project-operations')
      return jsonResponse(
        operationsData({
          cargo_dependencies: cargoData(),
          architecture: architectureData(),
        }),
      );
    if (url.pathname === '/api/document')
      return jsonResponse({
        path: architectureSource.path,
        title: '项目运作',
        navigation: emptyNavigation,
        content: '# 设计声明\n\n\n\n\n\n\n索引原始职责',
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
    await screen.findByRole('button', { name: '选择 Cargo 包 core' }),
  );
  expect(
    fetchMock.mock.calls.some(([input]) =>
      String(input).startsWith('/api/document?'),
    ),
  ).toBe(false);
  const source = screen.getByRole('button', {
    name: '资料索引，查看关联的 Vault 声明',
  });
  await user.click(source);
  expect(await screen.findByText('索引原始职责')).toBeInTheDocument();
  const request = fetchMock.mock.calls.find(([input]) =>
    String(input).startsWith('/api/document?'),
  );
  const query = new URL(String(request?.[0]), 'http://localhost').searchParams;
  expect(query.get('path')).toBe(architectureSource.path);
  expect(query.get('generation')).toBe('7');
  await user.click(screen.getByRole('button', { name: '关闭原文' }));
  await waitFor(() => expect(source).toHaveFocus());
});

it('重新核验卸载旧清单并清理选择/筛选，同时保留核验按钮焦点', async () => {
  let finish!: (response: Response) => void;
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(operationsData({ cargo_dependencies: cargoData() })),
    )
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(
    <ProjectOperations
      projects={fixture.projects}
      generation={7}
      onOpen={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  await user.click(
    await screen.findByRole('button', { name: '选择 Cargo 包 core' }),
  );
  await user.selectOptions(
    screen.getByRole('combobox', { name: '清单依赖种类' }),
    'dev',
  );
  const retry = screen.getByRole('button', { name: '重新核验目录' });
  retry.focus();
  await user.keyboard('{Enter}');
  expect(
    screen.queryByRole('group', { name: 'Cargo 包' }),
  ).not.toBeInTheDocument();
  expect(retry).toHaveFocus();
  await act(async () =>
    finish(jsonResponse(operationsData({ cargo_dependencies: cargoData() }))),
  );
  expect(
    await screen.findByRole('button', { name: '选择 Cargo 包 app' }),
  ).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByRole('combobox', { name: '清单依赖种类' })).toHaveValue(
    'all',
  );
  expect(retry).toHaveFocus();
});

it.each(['project', 'generation'] as const)(
  '换%s后不残留另一份清单选择',
  async (change) => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockImplementation(async (input) => {
        const query = new URL(String(input), 'http://localhost').searchParams;
        return jsonResponse(
          operationsData({
            project: query.get('project')!,
            generation: Number(query.get('generation')),
            cargo_dependencies: cargoData(),
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
    const { rerender } = render(
      <ProjectOperations {...props} generation={7} />,
    );
    await user.click(
      await screen.findByRole('button', { name: '选择 Cargo 包 core' }),
    );
    if (change === 'project')
      await user.selectOptions(
        screen.getByRole('combobox', { name: '项目' }),
        fixture.projects[1].root,
      );
    else rerender(<ProjectOperations {...props} generation={8} />);
    expect(
      await screen.findByRole('button', { name: '选择 Cargo 包 app' }),
    ).toHaveAttribute('aria-pressed', 'true');
  },
);

it('损坏的present Cargo响应显示可恢复错误，不能隐藏为缺少功能', async () => {
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ ...operationsData(), cargo_dependencies: null }),
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
    'Cargo 清单依赖数据格式或来源不完整',
  );
  expect(screen.queryByText('暂无 Cargo 清单依赖数据')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
});
