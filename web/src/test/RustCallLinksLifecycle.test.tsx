import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RustCallSitesPanel } from '../components/RustCallSitesPanel';
import { ProjectOperations } from '../components/ProjectOperations';
import { fixture, jsonResponse } from './fixtures';
import { operationsData } from './operationsFixtures';
import { rustAnalysis } from './rustCallLinksFixtures';

const sourceButton = '查看调用 读取 的词法关联，第 3 行字节列 5';

it.each(['project', 'generation', 'batch'] as const)(
  '换%s清除已选调用与定义，同秒新批不能复用旧详情',
  async (change) => {
    const data = operationsData({ rust_call_sites: rustAnalysis() });
    const { rerender } = render(
      <RustCallSitesPanel data={data} onOpen={vi.fn()} />,
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: sourceButton }));
    expect(
      screen.getByRole('combobox', { name: '同文件自由函数定义' }),
    ).toHaveValue('f0:context:130:200');
    const next = { ...data, rust_call_sites: rustAnalysis() };
    if (change === 'project') next.project = '项目/另一项目';
    if (change === 'generation') next.generation = 8;
    if (change === 'batch') {
      next.rust_call_sites.links!.batch_id = 'b'.repeat(64);
      next.rust_call_sites.graph!.contexts[2].name = '换名';
      for (const index of [0, 1, 4])
        next.rust_call_sites.graph!.calls[index].name = '换名';
    }
    rerender(<RustCallSitesPanel data={next} onOpen={vi.fn()} />);
    expect(
      screen.getByRole('combobox', { name: '同文件自由函数定义' }),
    ).toHaveValue('');
    expect(
      screen.queryByRole('heading', { name: '所选调用：读取' }),
    ).not.toBeInTheDocument();
    if (change === 'batch') {
      expect(next.rust_call_sites.observed_at).toBe(
        data.rust_call_sites!.observed_at,
      );
      expect(
        screen.getByRole('list', { name: 'Rust 调用位置' }),
      ).toHaveTextContent('换名');
    }
  },
);

it('切换文件往返清理关联选择，空文件可见零定义说明', async () => {
  render(
    <RustCallSitesPanel
      data={operationsData({ rust_call_sites: rustAnalysis() })}
      onOpen={vi.fn()}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: sourceButton }));
  await user.click(
    screen.getByRole('button', { name: '选择 Rust 文件 src/other.rs' }),
  );
  expect(
    screen.getByText('此文件没有可查看的自由函数定义。'),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: '所选函数的同批引用' }),
  ).not.toBeInTheDocument();
  await user.click(
    screen.getByRole('button', { name: '选择 Rust 文件 src/lib.rs' }),
  );
  expect(
    screen.getByRole('combobox', { name: '同文件自由函数定义' }),
  ).toHaveValue('');
});

it('重新核验卸载旧关联，响应同秒同批仍清理详情并保留核验焦点', async () => {
  let finish!: (response: Response) => void;
  const response = () =>
    jsonResponse(operationsData({ rust_call_sites: rustAnalysis() }));
  vi.stubGlobal(
    'fetch',
    vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(response())
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      ),
  );
  const user = userEvent.setup();
  render(
    <ProjectOperations
      projects={fixture.projects}
      generation={7}
      onOpen={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  await user.click(await screen.findByRole('button', { name: sourceButton }));
  const retry = screen.getByRole('button', { name: '重新核验目录' });
  retry.focus();
  await user.keyboard('{Enter}');
  expect(
    screen.queryByRole('region', { name: 'Rust 词法关联详情' }),
  ).not.toBeInTheDocument();
  expect(retry).toHaveFocus();
  await act(async () => finish(response()));
  expect(
    await screen.findByRole('combobox', { name: '同文件自由函数定义' }),
  ).toHaveValue('');
  expect(retry).toHaveFocus();
});

it('present损坏关联使整体响应失败，并提供重试而非旧服务降级', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async () =>
      jsonResponse(
        operationsData({
          rust_call_sites: { ...rustAnalysis(), links: null },
        }),
      ),
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
    'Rust 词法关联数据格式、批标识或引用不完整',
  );
  expect(
    screen.queryByRole('list', { name: 'Rust 调用位置' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
});
