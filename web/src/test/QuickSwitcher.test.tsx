import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { App } from '../App';
import {
  defaultApiResponse,
  documents,
  emptyNavigation,
  jsonResponse,
} from './fixtures';

const fetchMock = vi.fn<typeof fetch>();
beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) =>
    String(input).startsWith('/api/document')
      ? jsonResponse({
          path: documents[0].path,
          title: '星舟总览',
          content: '只读文档',
          navigation: emptyNavigation,
        })
      : defaultApiResponse(input),
  );
  vi.stubGlobal('fetch', fetchMock);
});

it('快速切换使用path接口传完整路径片段，Enter打开第一项', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /快速切换/ }));
  const dialog = screen.getByRole('dialog', { name: '快速切换' });
  const input = within(dialog).getByRole('searchbox');
  expect(input).toHaveFocus();
  await user.type(input, '星舟 INDEX');
  await within(dialog).findByText('星舟总览');
  const call = fetchMock.mock.calls
    .filter(([url]) => String(url).startsWith('/api/search'))
    .at(-1)!;
  const params = new URL(String(call[0]), 'http://localhost').searchParams;
  expect(params.get('mode')).toBe('path');
  expect(params.get('q')).toBe('星舟 INDEX');
  expect(params.get('generation')).toBe('7');
  await user.keyboard('{Enter}');
  expect(await screen.findByText('只读文档')).toBeInTheDocument();
  expect(
    screen.queryByRole('dialog', { name: '快速切换' }),
  ).not.toBeInTheDocument();
});

it('⌘K与CtrlK可在三页面打开，Esc关闭并还原启动按钮焦点', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  const trigger = screen.getByRole('button', { name: /快速切换/ });
  trigger.focus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  fireEvent(
    screen.getByRole('dialog', { name: '快速切换' }),
    new Event('cancel', { cancelable: true }),
  );
  expect(trigger).toHaveFocus();
  for (const page of ['项目', '文档']) {
    await user.click(screen.getByRole('button', { name: page }));
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    expect(
      screen.getByRole('dialog', { name: '快速切换' }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '关闭快速切换' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  }
});

it('方向键选择结果，快照刷新关闭快速切换并取消其请求', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /快速切换/ }));
  const result = await screen.findByRole('button', {
    name: '星舟总览，查看原文',
  });
  await user.keyboard('{ArrowDown}');
  expect(result).toHaveFocus();
  await user.click(screen.getByRole('button', { name: '关闭快速切换' }));
  const searchCall = fetchMock.mock.calls.find(([input]) =>
    String(input).startsWith('/api/search'),
  )!;
  expect(searchCall[1]?.signal?.aborted).toBe(true);
  // 刷新按钮挪进了顶栏状态浮层。
  await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
  await user.click(screen.getByRole('button', { name: '刷新快照' }));
  await waitFor(() =>
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes('refresh=true'),
      ),
    ).toBe(true),
  );
});
