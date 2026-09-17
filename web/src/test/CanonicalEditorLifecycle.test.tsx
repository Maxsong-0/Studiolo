import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { SessionBoundary } from '../SessionBoundary';
import { SessionClient } from '../sessionClient';
import { CanonicalEditor } from '../components/CanonicalEditor';
import { SessionDialog } from '../components/SessionDialog';
import { DraftContext } from '../draftContext';
import { DraftStore } from '../draftStore';
import { canonical, ownerScope } from './canonicalFixtures';
import {
  currentSession,
  mockSessions,
  pair,
  sessionResponse,
} from './sessionFixtures';
import { jsonResponse } from './fixtures';

// 生命周期只替换输入表面；CodeMirror 的真实事务/撤销在独立组件契约验证。
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

let scope: string | undefined;
beforeEach(() => {
  scope = ownerScope;
});
function service(document = canonical) {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: scope });
    if (String(input).startsWith('/api/v1/file?'))
      return jsonResponse(document);
    return sessionResponse(input, init);
  });
  return fetchMock;
}

it('会话锁定隐去草稿，同主人重新核对后人工恢复；整个页面离开有未保存提醒', async () => {
  const fetchMock = service();
  const client = new SessionClient();
  render(
    <SessionBoundary client={client}>
      {() => <CanonicalEditor path={canonical.path} onClose={vi.fn()} />}
    </SessionBoundary>,
  );
  const editor = await screen.findByRole('textbox', { name: 'Markdown 源码' });
  fireEvent.change(editor, { target: { value: '仅在内存中的草稿' } });
  const leaving = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(leaving);
  expect(leaving.defaultPrevented).toBe(true);
  act(() => client.suspend());
  expect(
    screen.queryByDisplayValue('仅在内存中的草稿'),
  ).not.toBeInTheDocument();
  act(() => client.resume());
  const restore = await screen.findByRole('button', { name: '恢复本页草稿' });
  expect(screen.getByRole('textbox', { name: 'Markdown 源码' })).toHaveValue(
    canonical.content,
  );
  fireEvent.click(restore);
  expect(screen.getByRole('textbox', { name: 'Markdown 源码' })).toHaveValue(
    '仅在内存中的草稿',
  );
  expect(
    fetchMock.mock.calls.some(([path]) => path === '/api/v1/mutations'),
  ).toBe(false);
});

it('新的 owner scope 不显示、恢复或导出旧主人草稿', async () => {
  service();
  const client = new SessionClient();
  render(
    <SessionBoundary client={client}>
      {() => <CanonicalEditor path={canonical.path} onClose={vi.fn()} />}
    </SessionBoundary>,
  );
  fireEvent.change(
    await screen.findByRole('textbox', { name: 'Markdown 源码' }),
    { target: { value: '旧主人未保存正文' } },
  );
  act(() => client.suspend());
  scope = 'e'.repeat(64);
  act(() => client.resume());
  expect(
    await screen.findByRole('textbox', { name: 'Markdown 源码' }),
  ).toHaveValue(canonical.content);
  expect(
    screen.queryByRole('button', { name: '恢复本页草稿' }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: '导出草稿与恢复信息' }),
  ).toBeDisabled();
});

it('旧会话无scope，即使服务标可编辑也保持只读', async () => {
  scope = undefined;
  service();
  render(
    <SessionBoundary>
      {() => <CanonicalEditor path={canonical.path} onClose={vi.fn()} />}
    </SessionBoundary>,
  );
  expect(
    await screen.findByRole('textbox', { name: 'Markdown 源码' }),
  ).toHaveAttribute('readonly');
  expect(
    screen.getByText('当前会话未提供可信主人范围，编辑保持只读。'),
  ).toBeInTheDocument();
});
it('超过256KiB仅显示只读文本，不启动编辑器或结构预览', async () => {
  service({
    ...canonical,
    content: '文'.repeat(90000),
    editing: { allowed: false, reason: 'too_large', max_bytes: 262144 },
  });
  render(
    <SessionBoundary>
      {() => <CanonicalEditor path={canonical.path} onClose={vi.fn()} />}
    </SessionBoundary>,
  );
  expect(
    await screen.findByRole('region', { name: '大文档只读原文' }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('textbox', { name: 'Markdown 源码' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: 'Markdown 预览' }),
  ).not.toBeInTheDocument();
});

it('取消关闭保留编辑内容；保留并关闭不发出保存', async () => {
  const fetchMock = service();
  const close = vi.fn();
  render(
    <SessionBoundary>
      {() => <CanonicalEditor path={canonical.path} onClose={close} />}
    </SessionBoundary>,
  );
  fireEvent.change(
    await screen.findByRole('textbox', { name: 'Markdown 源码' }),
    { target: { value: '保留的草稿' } },
  );
  fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
  expect(
    screen.getByRole('alertdialog', { name: '处理未保存草稿' }),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '继续编辑' }));
  expect(screen.getByRole('textbox', { name: 'Markdown 源码' })).toHaveValue(
    '保留的草稿',
  );
  expect(close).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
  fireEvent.click(screen.getByRole('button', { name: '保留内存草稿并关闭' }));
  expect(close).toHaveBeenCalledOnce();
  expect(
    fetchMock.mock.calls.some(([path]) => path === '/api/v1/mutations'),
  ).toBe(false);
});

it('显式退出前必须处理草稿，返回保留不会撤销会话', async () => {
  const fetchMock = service();
  const client = new SessionClient();
  await client.check();
  const store = new DraftStore().forScope(ownerScope);
  store.keep({ base: canonical, draft: '退出前草稿', pending: null });
  render(
    <DraftContext.Provider value={store}>
      <SessionDialog
        client={client}
        current={currentSession}
        busy={false}
        onClose={vi.fn()}
      />
    </DraftContext.Provider>,
  );
  fireEvent.click(screen.getByRole('button', { name: '退出登录' }));
  expect(
    screen.getByRole('alertdialog', { name: '退出前处理草稿' }),
  ).toBeInTheDocument();
  expect(
    fetchMock.mock.calls.some(([path]) => path === '/api/v1/sessions/revoke'),
  ).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: '返回并保留草稿' }));
  expect(store.getSnapshot()).toBe(1);
  fireEvent.click(screen.getByRole('button', { name: '退出登录' }));
  fireEvent.click(screen.getByRole('button', { name: '放弃内存草稿并退出' }));
  await waitFor(() => expect(client.getSnapshot().phase).toBe('signed-out'));
  expect(store.getSnapshot()).toBe(0);
  client.stop();
});
