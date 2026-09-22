import { StrictMode } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { SessionBoundary } from '../SessionBoundary';
import { REVISIT_INTERVAL_MS } from '../sessionClient';
import { jsonResponse } from './fixtures';
import {
  deferred,
  mockSessions,
  pair,
  sessionResponse,
} from './sessionFixtures';

function boundary() {
  return (
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>
  );
}

describe('会话入口', () => {
  it('StrictMode 引导未确认前不读取内容，明确 local 后允许旧工作台', async () => {
    const pending = deferred<Response>();
    const fetchMock = mockSessions();
    fetchMock.mockImplementation((input, init) =>
      String(input) === '/api/v1/bootstrap'
        ? pending.promise
        : Promise.resolve(sessionResponse(input, init)),
    );
    render(<StrictMode>{boundary()}</StrictMode>);
    expect(screen.getByRole('status')).toHaveTextContent('正在核对');
    expect(
      fetchMock.mock.calls.every(([input]) => input === '/api/v1/bootstrap'),
    ).toBe(true);
    await act(async () =>
      pending.resolve(jsonResponse({ authentication: 'local' })),
    );
    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: '账户与会话' }),
    ).not.toBeInTheDocument();
  });

  it('无会话先读取自身清理 Cookie，只在点击后签发，随后加载工作台', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/session'
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    const user = userEvent.setup();
    render(boundary());
    await screen.findByRole('button', { name: '登录工作台' });
    expect(fetchMock.mock.calls.map(([input]) => input)).toEqual([
      '/api/v1/bootstrap',
      '/api/v1/session',
    ]);
    await user.click(screen.getByRole('button', { name: '登录工作台' }));
    expect(await screen.findByRole('table')).toBeInTheDocument();
    const [, request] = fetchMock.mock.calls.find(
      ([input, init]) =>
        input === '/api/v1/sessions' && init?.method === 'POST',
    )!;
    expect(request).toMatchObject({
      body: '{}',
      credentials: 'same-origin',
      cache: 'no-store',
      redirect: 'error',
    });
    expect(request?.headers).toEqual({
      Accept: 'application/json',
      'Content-Type': 'application/json',
      'X-Studiolo-Request': '1',
    });
  });

  it('明确 access 引导保留 Access 模式，不访问应用会话端点', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/bootstrap'
        ? jsonResponse({ authentication: 'access' })
        : sessionResponse(input, init),
    );
    render(boundary());
    await screen.findByRole('table');
    expect(
      fetchMock.mock.calls.some(([input]) => input === '/api/v1/session'),
    ).toBe(false);
    expect(
      screen.queryByRole('button', { name: '账户与会话' }),
    ).not.toBeInTheDocument();
  });

  it('引导网络错误保留关闭状态，不自动重试或读取内容', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'));
    render(boundary());
    await screen.findByRole('alert');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByRole('button', { name: '登录工作台' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it.each([
    { status: 200, value: { authentication: 'unknown' } },
    { status: 200, value: { authentication: 'local', extra: true } },
    { status: 404, value: {} },
    { status: 401, value: {} },
  ])(
    '引导失败或格式异常不降级为未认证读取（$status）',
    async ({ status, value }) => {
      const fetchMock = mockSessions();
      fetchMock.mockResolvedValue(jsonResponse(value, status));
      render(boundary());
      await screen.findByRole('alert');
      expect(fetchMock).toHaveBeenCalledTimes(1);
      expect(screen.queryByRole('table')).not.toBeInTheDocument();
      expect(
        screen.queryByRole('button', { name: '登录工作台' }),
      ).not.toBeInTheDocument();
    },
  );

  it('内容读取 401 立即卸载旧项目与原文弹层，重新核对前不允许登录', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      String(input).startsWith('/api/document')
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    const user = userEvent.setup();
    render(boundary());
    await user.click(
      await screen.findByRole('button', { name: '验收只读工作台，查看原文' }),
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('登录已失效');
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByText('星舟')).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: '登录工作台' }),
    ).not.toBeInTheDocument();
  });

  it('登录 503 只提供显式核对；核对已有会话后恢复，不重试签发', async () => {
    const fetchMock = mockSessions();
    let hasSession = false;
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/session')
        return jsonResponse(hasSession ? pair : {}, hasSession ? 200 : 401);
      if (input === '/api/v1/sessions' && init?.method === 'POST') {
        hasSession = true;
        return jsonResponse({}, 503);
      }
      return sessionResponse(input, init);
    });
    const user = userEvent.setup();
    render(boundary());
    await user.click(await screen.findByRole('button', { name: '登录工作台' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '操作可能已生效',
    );
    window.dispatchEvent(new Event('focus'));
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '核对登录状态' }));
    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
  });

  it('一天内切回页面不再核对：不清内容、不重挂、不发核对请求（ADR-022）', async () => {
    const fetchMock = mockSessions();
    render(boundary());
    const table = await screen.findByRole('table');
    const before = fetchMock.mock.calls.length;
    for (const state of ['hidden', 'visible'] as const) {
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        get: () => state,
      });
      await act(async () =>
        document.dispatchEvent(new Event('visibilitychange')),
      );
    }
    await act(async () => window.dispatchEvent(new Event('focus')));
    // 同一个表格节点还在：没有卸载重挂，也没有任何新的请求。
    expect(screen.getByRole('table')).toBe(table);
    expect(fetchMock.mock.calls.length).toBe(before);
  });

  it('超过一天再切回页面才核对，过期会话清除已展示内容', async () => {
    const fetchMock = mockSessions();
    render(boundary());
    await screen.findByRole('table');
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/session'
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    const start = Date.now();
    vi.spyOn(Date, 'now').mockReturnValue(start + REVISIT_INTERVAL_MS - 1000);
    await act(async () => window.dispatchEvent(new Event('focus')));
    expect(screen.getByRole('table')).toBeInTheDocument();
    vi.spyOn(Date, 'now').mockReturnValue(start + REVISIT_INTERVAL_MS + 1000);
    await act(async () => window.dispatchEvent(new Event('focus')));
    await screen.findByRole('button', { name: '登录工作台' });
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('跨标签页会话变化重新核对，旧授权不保留', async () => {
    vi.stubGlobal('BroadcastChannel', undefined);
    const fetchMock = mockSessions();
    render(boundary());
    await screen.findByRole('table');
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/session'
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    await act(async () =>
      window.dispatchEvent(
        new StorageEvent('storage', {
          key: 'studiolo-session-change',
          newValue: 'changed',
        }),
      ),
    );
    await waitFor(() =>
      expect(screen.queryByRole('table')).not.toBeInTheDocument(),
    );
    await screen.findByRole('button', { name: '登录工作台' });
  });
});
