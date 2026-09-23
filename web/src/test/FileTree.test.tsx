import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { Documents } from '../components/Documents';
import { FileTree } from '../components/FileTree';
import { fixture, jsonResponse } from './fixtures';
import { treeEntry, treePage } from './treeFixtures';

const fetchMock = vi.fn<typeof fetch>();
const onOpen = vi.fn();
const onRefresh = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  onOpen.mockReset();
  onRefresh.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});

function renderTree() {
  return render(
    <FileTree generation={7} onOpen={onOpen} onRefresh={onRefresh} />,
  );
}

it('默认搜索不请求文件树；展开才读子目录，附件只显示文本和精确元数据', async () => {
  const attachment = treeEntry(
    '纯附件/中文很长的附件 <img src=x onerror=alert(1)>.pdf',
  );
  fetchMock.mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/search')
      return jsonResponse({ generation: 7, hits: [], truncated: false });
    if (url.searchParams.get('path') === '')
      return jsonResponse(
        treePage('', [
          treeEntry('纯附件', 'directory'),
          treeEntry('说明.md', 'markdown'),
        ]),
      );
    return jsonResponse(treePage('纯附件', [attachment]));
  });
  const user = userEvent.setup();
  const { container } = render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  await screen.findByText('当前快照没有可读取的文档。');
  expect(
    fetchMock.mock.calls.every(([url]) =>
      String(url).startsWith('/api/search?'),
    ),
  ).toBe(true);
  await user.click(screen.getByRole('tab', { name: '文件树' }));
  const directory = await screen.findByRole('button', {
    name: '展开目录 纯附件',
  });
  expect(fetchMock).toHaveBeenCalledTimes(2);
  await user.click(directory);
  const attachmentButton = await screen.findByRole('button', {
    name: `${attachment.name}，查看附件信息`,
  });
  expect(fetchMock).toHaveBeenCalledTimes(3);
  expect(directory).toHaveAttribute('aria-expanded', 'true');
  await user.click(attachmentButton);
  const dialog = screen.getByRole('dialog', { name: '附件信息' });
  expect(within(dialog).getByText(attachment.path)).toBeInTheDocument();
  expect(within(dialog).getByText('9007199254740993 字节')).toBeInTheDocument();
  expect(within(dialog).getByText('.pdf')).toBeInTheDocument();
  expect(within(dialog).getByText(/不提供预览或下载/)).toBeInTheDocument();
  expect(container.querySelector('img')).toBeNull();
  expect(container.querySelector('a')).toBeNull();
  expect(fetchMock).toHaveBeenCalledTimes(3);
  await user.click(screen.getByRole('button', { name: '关闭附件信息' }));
  expect(attachmentButton).toHaveFocus();
  await user.click(screen.getByRole('button', { name: '说明.md，查看原文' }));
  expect(onOpen).toHaveBeenCalledWith({ path: '说明.md', line: 1 });
  await user.click(directory);
  expect(
    screen.queryByRole('button', { name: `${attachment.name}，查看附件信息` }),
  ).not.toBeInTheDocument();
});

it('分页替换200项并保留分页焦点，上一页可回到目录起点', async () => {
  const first = Array.from({ length: 200 }, (_, index) =>
    treeEntry(`附件${index}.pdf`),
  );
  fetchMock.mockImplementation(async (input) => {
    const offset = new URL(String(input), 'http://localhost').searchParams.get(
      'offset',
    );
    return jsonResponse(
      offset === '200'
        ? treePage('', [treeEntry('末尾.pdf')], { offset: 200, total: 201 })
        : treePage('', first, { total: 201, next_offset: 200 }),
    );
  });
  const user = userEvent.setup();
  renderTree();
  await screen.findByRole('button', { name: '附件0.pdf，查看附件信息' });
  const next = screen.getByRole('button', { name: '下一页' });
  await user.click(next);
  await screen.findByRole('button', { name: '末尾.pdf，查看附件信息' });
  expect(
    screen.queryByRole('button', { name: '附件0.pdf，查看附件信息' }),
  ).not.toBeInTheDocument();
  expect(next).toHaveFocus();
  expect(next).toHaveAttribute('aria-disabled', 'true');
  expect(screen.getByText('201–201 / 201 项')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '上一页' }));
  await screen.findByRole('button', { name: '附件0.pdf，查看附件信息' });
  expect(
    screen.queryByRole('button', { name: '末尾.pdf，查看附件信息' }),
  ).not.toBeInTheDocument();
});

it('目录错误可重试；过期快照仅提供刷新入口，空根和空目录有说明', async () => {
  const user = userEvent.setup();
  fetchMock.mockResolvedValueOnce(jsonResponse({ error: '稍后再试' }, 429));
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage('', [treeEntry('空目录', 'directory')])),
  );
  fetchMock.mockResolvedValueOnce(jsonResponse(treePage('空目录', [])));
  renderTree();
  expect(await screen.findByRole('alert')).toHaveTextContent('稍后再试');
  await user.click(screen.getByRole('button', { name: '重试目录' }));
  await user.click(
    await screen.findByRole('button', { name: '展开目录 空目录' }),
  );
  expect(await screen.findByText('此目录为空。')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '折叠目录 空目录' }));
  fetchMock.mockResolvedValueOnce(jsonResponse({ error: 'stale' }, 409));
  await user.click(screen.getByRole('button', { name: '展开目录 空目录' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('快照已更新');
  await user.click(screen.getByRole('button', { name: '刷新工作台' }));
  expect(onRefresh).toHaveBeenCalledOnce();
});

it('空根不会虚构目录或附件', async () => {
  fetchMock.mockResolvedValue(jsonResponse(treePage('', [])));
  renderTree();
  expect(
    await screen.findByText('当前快照没有可显示的文件或目录。'),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: /查看附件信息/ }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: '下一页' }),
  ).not.toBeInTheDocument();
});

it('键盘可展开目录、打开附件并取消弹窗回到原焦点', async () => {
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage('', [treeEntry('资料', 'directory')])),
  );
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage('资料', [treeEntry('资料/README')])),
  );
  const user = userEvent.setup();
  renderTree();
  const directory = await screen.findByRole('button', {
    name: '展开目录 资料',
  });
  await user.tab();
  expect(directory).toHaveFocus();
  await user.keyboard('{Enter}');
  const attachment = await screen.findByRole('button', {
    name: 'README，查看附件信息',
  });
  await user.tab();
  expect(attachment).toHaveFocus();
  await user.keyboard('{Enter}');
  const dialog = screen.getByRole('dialog', { name: '附件信息' });
  expect(within(dialog).getByText('无后缀')).toBeInTheDocument();
  fireEvent(dialog, new Event('cancel', { cancelable: true }));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(attachment).toHaveFocus();
});

it('折叠取消子目录请求，迟到内容不可出现，重新展开可恢复', async () => {
  let finishOld!: (response: Response) => void;
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage('', [treeEntry('资料', 'directory')])),
  );
  fetchMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve;
      }),
  );
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage('资料', [treeEntry('资料/最新.pdf')])),
  );
  const user = userEvent.setup();
  renderTree();
  await user.click(
    await screen.findByRole('button', { name: '展开目录 资料' }),
  );
  expect(screen.getByRole('status')).toHaveTextContent('正在读取目录');
  await user.click(screen.getByRole('button', { name: '折叠目录 资料' }));
  expect(fetchMock.mock.calls[1][1]?.signal?.aborted).toBe(true);
  await user.click(screen.getByRole('button', { name: '展开目录 资料' }));
  await screen.findByRole('button', { name: '最新.pdf，查看附件信息' });
  await act(async () =>
    finishOld(jsonResponse(treePage('资料', [treeEntry('资料/过时.pdf')]))),
  );
  expect(
    screen.queryByRole('button', { name: '过时.pdf，查看附件信息' }),
  ).not.toBeInTheDocument();
});

it('切换代次立即卸载旧附件信息并丢弃旧请求', async () => {
  fetchMock.mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    const generation = Number(url.searchParams.get('generation'));
    if (url.pathname === '/api/search')
      return jsonResponse({ generation, hits: [], truncated: false });
    return jsonResponse(
      treePage('', generation === 7 ? [treeEntry('旧附件.pdf')] : [], {
        generation,
      }),
    );
  });
  const user = userEvent.setup();
  const documents = (generation: number) => (
    <Documents
      projects={fixture.projects}
      generation={generation}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />
  );
  const { rerender } = render(documents(7));
  await user.click(screen.getByRole('tab', { name: '文件树' }));
  await user.click(
    await screen.findByRole('button', { name: '旧附件.pdf，查看附件信息' }),
  );
  expect(screen.getByRole('dialog')).toBeInTheDocument();
  rerender(documents(8));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(screen.queryByText('旧附件.pdf')).not.toBeInTheDocument();
  expect(
    await screen.findByText('当前快照没有可显示的文件或目录。'),
  ).toBeInTheDocument();
});
