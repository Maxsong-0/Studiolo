import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ProjectOperations } from '../components/ProjectOperations';
import { fixture, jsonResponse } from './fixtures';
import { operationsData } from './operationsFixtures';
import { rustData } from './rustCallFixtures';

const props = {
  projects: fixture.projects,
  onOpen: vi.fn(),
  onRefresh: vi.fn(),
};
const response = () =>
  jsonResponse(operationsData({ rust_call_sites: rustData() }));

it.each(['success', 'error'] as const)(
  '核验pending→%s保留按钮焦点、卸载旧源码证据并清理选项',
  async (result) => {
    let finish!: (response: Response) => void;
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response())
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      )
      .mockImplementation(async () => response());
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<ProjectOperations {...props} generation={7} />);
    await user.click(
      await screen.findByRole('button', {
        name: '选择 Rust 文件 src/empty.rs',
      }),
    );
    await user.selectOptions(
      screen.getByRole('combobox', { name: '调用语法种类' }),
      'macro',
    );
    const retry = screen.getByRole('button', { name: '重新核验目录' });
    retry.focus();
    await user.keyboard('{Enter}');
    expect(retry).toHaveFocus();
    expect(retry).toHaveAttribute('aria-disabled', 'true');
    expect(
      screen.queryByRole('group', { name: 'Rust 白名单文件' }),
    ).not.toBeInTheDocument();
    await user.keyboard('{Enter}');
    expect(fetchMock).toHaveBeenCalledTimes(2);
    await act(async () =>
      finish(
        result === 'success' ? response() : new Response('{}', { status: 503 }),
      ),
    );
    if (result === 'error') {
      expect(await screen.findByRole('alert')).toBeInTheDocument();
      expect(retry).toHaveFocus();
      expect(retry).toHaveAccessibleName('重试');
      await user.keyboard('{Enter}');
    }
    expect(
      await screen.findByRole('button', {
        name: '选择 Rust 文件 crates/core/internal/lib.rs',
      }),
    ).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('combobox', { name: '调用语法种类' })).toHaveValue(
      'all',
    );
    expect(screen.getByRole('button', { name: '全部上下文' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(retry).toHaveFocus();
  },
);

it.each(['project', 'generation'] as const)(
  '换%s清理文件、上下文和调用种类选择',
  async (change) => {
    vi.stubGlobal(
      'fetch',
      vi.fn<typeof fetch>().mockImplementation(async (input) => {
        const query = new URL(String(input), 'http://localhost').searchParams;
        return jsonResponse(
          operationsData({
            project: query.get('project')!,
            generation: Number(query.get('generation')),
            rust_call_sites: rustData(),
          }),
        );
      }),
    );
    const user = userEvent.setup();
    const { rerender } = render(
      <ProjectOperations {...props} generation={7} />,
    );
    await user.click(
      await screen.findByRole('button', {
        name: '选择 Rust 文件 src/empty.rs',
      }),
    );
    await user.selectOptions(
      screen.getByRole('combobox', { name: '调用语法种类' }),
      'method',
    );
    await user.click(screen.getByRole('button', { name: /文件根.*所属/ }));
    if (change === 'project')
      await user.selectOptions(
        screen.getByRole('combobox', { name: '项目' }),
        fixture.projects[1].root,
      );
    else rerender(<ProjectOperations {...props} generation={8} />);
    expect(
      await screen.findByRole('button', {
        name: '选择 Rust 文件 crates/core/internal/lib.rs',
      }),
    ).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('combobox', { name: '调用语法种类' })).toHaveValue(
      'all',
    );
    expect(screen.getByRole('button', { name: '全部上下文' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  },
);
