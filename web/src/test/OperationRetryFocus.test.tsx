import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ProjectOperations } from '../components/ProjectOperations';
import { fixture, jsonResponse } from './fixtures';
import { operationsData } from './operationsFixtures';

it.each([200, 503, 409])(
  '键盘重新核验在等待和%s响应后保留同一控制的焦点',
  async (status) => {
    let finish!: (response: Response) => void;
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(operationsData()))
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      );
    vi.stubGlobal('fetch', fetchMock);
    const onRefresh = vi.fn();
    const user = userEvent.setup();
    render(
      <ProjectOperations
        projects={fixture.projects}
        generation={7}
        onOpen={vi.fn()}
        onRefresh={onRefresh}
      />,
    );
    await screen.findByRole('region', { name: '目录与模块职责图' });
    const control = screen.getByRole('button', { name: '重新核验目录' });
    control.focus();
    await user.keyboard('{Enter}');
    expect(await screen.findByRole('status')).toHaveTextContent(
      '正在读取项目声明并核验目录',
    );
    expect(control).toHaveFocus();
    expect(control).toHaveAttribute('aria-disabled', 'true');
    expect(control).not.toBeDisabled();
    expect(
      screen.queryByRole('region', { name: '目录与模块职责图' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('list', { name: '目录比对结果' }),
    ).not.toBeInTheDocument();
    await user.keyboard('{Enter} ');
    expect(fetchMock).toHaveBeenCalledTimes(2);
    await act(async () =>
      finish(
        status === 200
          ? jsonResponse(operationsData())
          : jsonResponse({ error: '本次核验失败' }, status),
      ),
    );
    if (status === 200)
      await screen.findByRole('region', { name: '目录与模块职责图' });
    else
      expect(await screen.findByRole('alert')).toHaveTextContent(
        '本次核验失败',
      );
    expect(control).toHaveFocus();
    expect(control).toHaveAttribute('aria-disabled', String(status === 409));
    if (status === 409) {
      await user.keyboard('{Enter}');
      expect(fetchMock).toHaveBeenCalledTimes(2);
      await user.click(screen.getByRole('button', { name: '刷新工作台' }));
      expect(onRefresh).toHaveBeenCalledOnce();
    }
  },
);

it('错误重试在等待和恢复成功后保留焦点', async () => {
  let finish!: (response: Response) => void;
  vi.stubGlobal(
    'fetch',
    vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse({ error: '暂不可用' }, 503))
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
  await screen.findByRole('alert');
  const control = screen.getByRole('button', { name: '重试' });
  control.focus();
  await user.keyboard('{Enter}');
  expect(control).toHaveFocus();
  expect(control).toHaveAttribute('aria-disabled', 'true');
  await act(async () => finish(jsonResponse(operationsData())));
  await screen.findByRole('region', { name: '目录与模块职责图' });
  expect(control).toHaveFocus();
  expect(control).toHaveAccessibleName('重新核验目录');
});
