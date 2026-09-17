import { useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useFileOperationStore } from '../fileOperationContext';
import { SessionBoundary } from '../SessionBoundary';
import { SessionClient } from '../sessionClient';
import { CanonicalEditor } from '../components/CanonicalEditor';
import { canonical, ownerScope } from './canonicalFixtures';
import { deleting } from './fileOperationFixtures';
import { historyEntry, historyFile, historyPage } from './historyFixtures';
import { jsonResponse } from './fixtures';
import { mockSessions, pair, sessionResponse } from './sessionFixtures';

// 只替换重型输入表面；Controller、共享Store、SessionClient、历史读取均使用真实代码。
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

const otherPath = '笔记/另一篇干净页面.md';
function Surface() {
  const { store, record } = useFileOperationStore();
  const [path, setPath] = useState(canonical.path);
  return (
    <>
      <button
        onClick={() =>
          void store.submit(
            { ...deleting, path: '笔记/独立删除请求.md' },
            new AbortController().signal,
          )
        }
      >
        模拟另一入口的文件操作
      </button>
      <button onClick={() => setPath(otherPath)}>打开另一篇干净页面</button>
      <output aria-label="共享文件操作阶段">{record?.phase ?? 'none'}</output>
      <CanonicalEditor path={path} onClose={() => {}} />
    </>
  );
}

it('共享未知原请求阻断已排自动保存及另一干净页面的历史恢复入口', async () => {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    const url = new URL(String(input), 'https://fixture.invalid');
    const path = url.searchParams.get('path') ?? canonical.path;
    if (url.pathname === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (url.pathname === '/api/v1/file-operations') {
      if (init?.method === 'POST') throw new TypeError('文件操作的响应丢失');
      return jsonResponse({ enabled: true, vault_uuid: canonical.vault_uuid });
    }
    if (url.pathname === '/api/v1/file')
      return jsonResponse({ ...canonical, path });
    if (url.pathname === '/api/v1/history/file')
      return jsonResponse({ ...historyFile, path });
    if (url.pathname === '/api/v1/history')
      return jsonResponse({ ...historyPage, path });
    if (url.pathname === '/api/v1/mutations')
      return jsonResponse({ error: '该请求不应发出' }, 403);
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  const view = render(
    <SessionBoundary client={client}>{() => <Surface />}</SessionBoundary>,
  );
  const contentPosts = () =>
    fetchMock.mock.calls.filter(
      ([url, init]) => url === '/api/v1/mutations' && init?.method === 'POST',
    );
  try {
    const input = await screen.findByRole('textbox', { name: 'Markdown 源码' });
    vi.useFakeTimers();
    fireEvent.change(input, { target: { value: '已排定自动保存的草稿' } });
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: '模拟另一入口的文件操作' }),
      );
    });
    expect(screen.getByLabelText('共享文件操作阶段')).toHaveTextContent(
      'uncertain',
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(contentPosts()).toHaveLength(0);
    expect(input).toHaveValue('已排定自动保存的草稿');
    expect(input).toHaveAttribute('readonly');

    vi.useRealTimers();
    fireEvent.click(screen.getByRole('button', { name: '打开另一篇干净页面' }));
    expect(
      await screen.findByRole('textbox', { name: 'Markdown 源码' }),
    ).toHaveValue(canonical.content);
    fireEvent.click(screen.getByRole('button', { name: '页面历史' }));
    fireEvent.click(
      await screen.findByRole('button', {
        name: `查看版本 ${historyEntry.revision}`,
      }),
    );
    const restore = await screen.findByRole('button', {
      name: '恢复此历史版本…',
    });
    expect(restore).toBeDisabled();
    fireEvent.click(restore);
    expect(
      screen.queryByRole('alertdialog', { name: '确认恢复历史版本' }),
    ).not.toBeInTheDocument();
    expect(contentPosts()).toHaveLength(0);
    expect(screen.getByLabelText('共享文件操作阶段')).toHaveTextContent(
      'uncertain',
    );
  } finally {
    view.unmount();
    client.stop();
    vi.useRealTimers();
  }
});
