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

/** 动作面板发出的全文搜索（按搜索词挑出来，不与页面自己的请求混在一起）。 */
function searchCalls(query: string) {
  return fetchMock.mock.calls.filter(([url]) => {
    const path = String(url);
    return (
      path.startsWith('/api/search') &&
      new URL(path, 'http://localhost').searchParams.get('q') === query
    );
  });
}

/** 面板发出的全部全文搜索。 */
function paletteSearches() {
  return fetchMock.mock.calls.filter(([url]) => {
    const path = String(url);
    return (
      path.startsWith('/api/search') &&
      new URL(path, 'http://localhost').searchParams.get('mode') === 'full_text'
    );
  });
}

it('动作面板的文档组用全文接口带当前代次查询，停手才发一次，Enter 打开选中的文档', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /^动作面板/ }));
  const dialog = screen.getByRole('dialog', { name: '动作面板' });
  const input = within(dialog).getByRole('combobox', {
    name: '跳转、搜索或执行动作',
  });
  expect(input).toHaveFocus();
  // 这个词不命中任何动作或跳转，第一行就是文档。
  await user.type(input, '原文正文');
  const option = await within(dialog).findByRole('option', {
    name: /星舟总览/,
  });
  expect(option).toHaveAttribute('aria-selected', 'true');
  // 逐字输入只在停手后发一次；一个字时不搜。
  expect(paletteSearches()).toHaveLength(1);
  const params = new URL(String(paletteSearches()[0][0]), 'http://localhost')
    .searchParams;
  expect(params.get('mode')).toBe('full_text');
  expect(params.get('q')).toBe('原文正文');
  expect(params.get('generation')).toBe('7');
  expect(params.has('project')).toBe(false);
  await user.keyboard('{Enter}');
  expect(await screen.findByText('只读文档')).toBeInTheDocument();
  expect(
    screen.queryByRole('dialog', { name: '动作面板' }),
  ).not.toBeInTheDocument();
});

it('⌘K与CtrlK可在三页面打开，Esc关闭并还原启动按钮焦点', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  const trigger = screen.getByRole('button', { name: /^动作面板/ });
  trigger.focus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  fireEvent(
    screen.getByRole('dialog', { name: '动作面板' }),
    new Event('cancel', { cancelable: true }),
  );
  expect(trigger).toHaveFocus();
  for (const page of ['项目', '文档']) {
    await user.click(screen.getByRole('button', { name: page }));
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    expect(
      screen.getByRole('dialog', { name: '动作面板' }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '关闭动作面板' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  }
});

it('方向键选择结果，快照刷新关闭动作面板并取消其请求', async () => {
  const user = userEvent.setup();
  render(<App />);
  // 数据来源标签收进了状态浮层；等驾驶舱的项目表出来即工作台已读到。
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /^动作面板/ }));
  const dialog = screen.getByRole('dialog', { name: '动作面板' });
  const input = within(dialog).getByRole('combobox', {
    name: '跳转、搜索或执行动作',
  });
  // 「刷新」命中动作「刷新快照」，文档组再给出一条命中：两行之间用方向键移动。
  await user.type(input, '刷新');
  const result = await within(dialog).findByRole('option', {
    name: /星舟总览/,
  });
  expect(
    within(dialog).getByRole('option', { name: /刷新快照/ }),
  ).toHaveAttribute('aria-selected', 'true');
  await user.keyboard('{ArrowDown}');
  expect(result).toHaveAttribute('aria-selected', 'true');
  expect(input).toHaveAttribute('aria-activedescendant', result.id);
  expect(input).toHaveFocus();
  await user.click(screen.getByRole('button', { name: '关闭动作面板' }));
  const [searchCall] = searchCalls('刷新');
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
