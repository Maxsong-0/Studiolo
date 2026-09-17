import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ProjectOperations } from '../components/ProjectOperations';
import { fixture, jsonResponse } from './fixtures';
import { directorySource, operationsData } from './operationsFixtures';

const { renderDiagram } = vi.hoisted(() => ({ renderDiagram: vi.fn() }));
vi.mock('../components/OperationDiagram', () => ({
  OperationDiagram: renderDiagram,
}));

beforeEach(() => {
  renderDiagram.mockReset().mockImplementation(() => {
    throw new Error('<svg onload="alert(1)">渲染失败');
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>().mockImplementation(async (input) => {
      const query = new URL(String(input), 'http://localhost').searchParams;
      return jsonResponse(
        operationsData({
          project: query.get('project')!,
          generation: Number(query.get('generation')),
          directories: [
            { path: 'crates', label: '隐含父', source: null },
            {
              path: 'crates/core',
              label: '<img src=x onerror="alert(2)">目录职责',
              source: directorySource,
            },
          ],
        }),
      );
    }),
  );
});

afterEach(() => vi.restoreAllMocks());

it('图形意外渲染失败展示原因和纯文本数据，保留职责来源与目录漂移', async () => {
  const onOpen = vi.fn();
  render(
    <ProjectOperations
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={vi.fn()}
    />,
  );
  const fallback = await screen.findByRole('region', {
    name: '目录与模块职责文本数据',
  });
  expect(within(fallback).getByRole('alert')).toHaveTextContent(
    '<svg onload="alert(1)">渲染失败',
  );
  expect(
    within(fallback).getByText('<img src=x onerror="alert(2)">目录职责'),
  ).toBeInTheDocument();
  expect(
    within(fallback).getByText('路径推导的父目录 · 未声明职责'),
  ).toBeInTheDocument();
  expect(fallback.querySelector('img, svg, script')).toBeNull();
  await userEvent.click(
    within(fallback).getByRole('button', {
      name: 'crates/core，从文本查看职责声明来源',
    }),
  );
  expect(onOpen).toHaveBeenCalledWith(directorySource);
  expect(
    screen.getByRole('list', { name: '目录比对结果' }),
  ).toBeInTheDocument();
});

it.each(['project', 'generation'] as const)(
  '%s改变后重新尝试图形渲染',
  async (change) => {
    const props = {
      projects: fixture.projects,
      onOpen: vi.fn(),
      onRefresh: vi.fn(),
    };
    const { rerender } = render(
      <ProjectOperations {...props} generation={7} />,
    );
    await screen.findByRole('region', { name: '目录与模块职责文本数据' });
    renderDiagram.mockImplementation(() => <p>图形已恢复</p>);
    if (change === 'generation')
      rerender(<ProjectOperations {...props} generation={8} />);
    else
      await userEvent.selectOptions(
        screen.getByRole('combobox', { name: '项目' }),
        fixture.projects[1].root,
      );
    expect(await screen.findByText('图形已恢复')).toBeInTheDocument();
    expect(
      screen.queryByRole('region', { name: '目录与模块职责文本数据' }),
    ).not.toBeInTheDocument();
  },
);
