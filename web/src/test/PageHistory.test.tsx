import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CanonicalEditor } from '../components/CanonicalEditor';
import { DraftContext } from '../draftContext';
import { DraftStore } from '../draftStore';
import { installCanonicalRestorer } from '../canonicalApi';
import type { Restoration } from '../canonicalModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { historyEntry, historyFile, historyPage } from './historyFixtures';
import { jsonResponse } from './fixtures';

vi.mock('../components/MarkdownEditor', () => ({
  MarkdownEditor: ({
    value,
    readOnly,
    onChange,
  }: {
    value: string;
    readOnly: boolean;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown 源码"
      value={value}
      readOnly={readOnly}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

function service() {
  let disk = canonical;
  const read = vi.fn(async (url: string) => {
    if (url.startsWith('/api/v1/history/file?'))
      return jsonResponse(historyFile);
    if (url.startsWith('/api/v1/history?')) return jsonResponse(historyPage);
    return jsonResponse(disk);
  });
  vi.stubGlobal('fetch', read);
  return {
    read,
    setDisk: (value: typeof disk) => {
      disk = value;
    },
  };
}
function show(store = new DraftStore().forScope(ownerScope)) {
  return render(
    <DraftContext.Provider value={store}>
      <CanonicalEditor path={canonical.path} onClose={vi.fn()} />
    </DraftContext.Provider>,
  );
}
async function openVersion() {
  fireEvent.click(screen.getByRole('button', { name: '页面历史' }));
  fireEvent.click(
    await screen.findByRole('button', {
      name: `查看版本 ${historyEntry.revision}`,
    }),
  );
  return screen.findByRole('region', { name: '历史版本比较' });
}
it('历史主体按提交/归档区分；恢复需明确确认，取消不写、确认只发 restore', async () => {
  const server = service();
  const restore = vi.fn(async (request: Restoration) => {
    server.setDisk({ ...canonical, ...historyFile });
    return committed(request, historyFile.revision);
  });
  const uninstall = installCanonicalRestorer(restore);
  try {
    show();
    await screen.findByRole('textbox', { name: 'Markdown 源码' });
    const comparison = await openVersion();
    const entry = screen
      .getByRole('button', { name: `查看版本 ${historyEntry.revision}` })
      .closest('li')!;
    expect(
      within(entry).getByText('提交主体').nextElementSibling,
    ).toHaveTextContent('未知');
    expect(
      within(entry).getByText('归档主体').nextElementSibling,
    ).toHaveTextContent('虚构归档者');
    expect(screen.getByText(/页历史不等于灾难备份/)).toBeInTheDocument();
    expect(comparison.querySelector('script')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '恢复此历史版本…' }));
    fireEvent.click(screen.getByRole('button', { name: '取消恢复' }));
    expect(restore).not.toHaveBeenCalled();
    expect(
      screen.getByRole('button', { name: '恢复此历史版本…' }),
    ).toHaveFocus();
    fireEvent.click(screen.getByRole('button', { name: '恢复此历史版本…' }));
    fireEvent.keyDown(
      screen.getByRole('alertdialog', { name: '确认恢复历史版本' }),
      { key: 'Escape' },
    );
    expect(
      screen.queryByRole('alertdialog', { name: '确认恢复历史版本' }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '恢复此历史版本…' }));
    fireEvent.click(screen.getByRole('button', { name: '确认恢复此版本' }));
    await waitFor(() =>
      expect(
        screen.getByRole('textbox', { name: 'Markdown 源码' }),
      ).toHaveValue(historyFile.content),
    );
    expect(restore).toHaveBeenCalledOnce();
    expect(restore.mock.calls[0][0]).toMatchObject({
      kind: 'restore',
      source_revision: historyFile.revision,
    });
    expect(restore.mock.calls[0][0]).not.toHaveProperty('content');
    expect(screen.getByRole('button', { name: '隐藏页面历史' })).toHaveFocus();
  } finally {
    uninstall();
  }
});
it('历史浏览不替换输入内核；未处理草稿禁止恢复，关闭历史清选择并保留草稿', async () => {
  service();
  const store = new DraftStore().forScope(ownerScope);
  show(store);
  const input = await screen.findByRole('textbox', { name: 'Markdown 源码' });
  fireEvent.change(input, { target: { value: '绝不能被历史覆盖的草稿' } });
  await openVersion();
  expect(
    screen.getByRole('button', { name: '恢复此历史版本…' }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '关闭历史' }));
  expect(
    screen.queryByRole('region', { name: '历史版本比较' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('textbox', { name: 'Markdown 源码' })).toBe(input);
  expect(input).toHaveValue('绝不能被历史覆盖的草稿');
  expect(store.get(canonical)?.draft).toBe('绝不能被历史覆盖的草稿');
  expect(screen.getByRole('button', { name: '页面历史' })).toHaveFocus();
  fireEvent.click(screen.getByRole('button', { name: '页面历史' }));
  await screen.findByRole('button', {
    name: `查看版本 ${historyEntry.revision}`,
  });
  expect(
    screen.queryByRole('region', { name: '历史版本比较' }),
  ).not.toBeInTheDocument();
});
it('换主人或路径关闭旧历史；新作用域重新读取，旧版本内容不在界面保留', async () => {
  service();
  const root = new DraftStore();
  const view = render(
    <DraftContext.Provider value={root.forScope(ownerScope)}>
      <CanonicalEditor path={canonical.path} onClose={vi.fn()} />
    </DraftContext.Provider>,
  );
  await screen.findByRole('textbox', { name: 'Markdown 源码' });
  await openVersion();
  view.rerender(
    <DraftContext.Provider value={root.forScope('e'.repeat(64))}>
      <CanonicalEditor path={canonical.path} onClose={vi.fn()} />
    </DraftContext.Provider>,
  );
  expect(
    screen.queryByRole('region', { name: '页面历史' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: '历史版本比较' }),
  ).not.toBeInTheDocument();
  await screen.findByRole('textbox', { name: 'Markdown 源码' });
  view.rerender(
    <DraftContext.Provider value={root.forScope('e'.repeat(64))}>
      <CanonicalEditor path="另一页.md" onClose={vi.fn()} />
    </DraftContext.Provider>,
  );
  expect(
    screen.queryByRole('textbox', { name: 'Markdown 源码' }),
  ).not.toBeInTheDocument();
  await screen.findByText(/权威文档响应格式无效/);
});
it('大条目仅列元数据，选择按钮禁用且没有正文请求', async () => {
  const read = vi.fn(async (url: string) =>
    url.startsWith('/api/v1/history?')
      ? jsonResponse({
          ...historyPage,
          entries: [{ ...historyEntry, bytes: 262145 }],
        })
      : jsonResponse(canonical),
  );
  vi.stubGlobal('fetch', read);
  show();
  await screen.findByRole('textbox', { name: 'Markdown 源码' });
  fireEvent.click(screen.getByRole('button', { name: '页面历史' }));
  expect(
    await screen.findByRole('button', {
      name: `查看版本 ${historyEntry.revision}`,
    }),
  ).toBeDisabled();
  expect(screen.getByText(/仅列出元数据/)).toBeInTheDocument();
  await act(async () => {});
  expect(
    read.mock.calls.some(([url]) => url.startsWith('/api/v1/history/file?')),
  ).toBe(false);
});
