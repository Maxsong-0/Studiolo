import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../App';
import { SessionBoundary } from '../SessionBoundary';
import {
  csrf,
  currentSession,
  mockSessions,
  otherSession,
  sessionResponse,
} from './sessionFixtures';
import { jsonResponse } from './fixtures';

function renderWorkbench() {
  return render(
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>,
  );
}

describe('账户与会话', () => {
  it('按需列出元信息，吊销指定会话携带内存 CSRF，更新真实列表', async () => {
    const fetchMock = mockSessions();
    let revoked = false;
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/sessions/revoke') revoked = true;
      if (input === '/api/v1/sessions' && init?.method === 'GET') {
        return jsonResponse({
          sessions: [
            currentSession,
            { ...otherSession, state: revoked ? 'revoked' : 'active' },
          ],
        });
      }
      return sessionResponse(input, init);
    });
    const user = userEvent.setup();
    renderWorkbench();
    await screen.findByRole('table');
    expect(
      fetchMock.mock.calls.some(([input]) => input === '/api/v1/sessions'),
    ).toBe(false);
    await user.click(screen.getByRole('button', { name: '账户与会话' }));
    expect(
      await screen.findByRole('list', { name: '会话记录' }),
    ).toBeInTheDocument();
    expect(screen.getByText('此浏览器')).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(currentSession.id);
    expect(document.body.textContent).not.toContain(csrf);
    await user.click(screen.getByRole('button', { name: '退出会话 2' }));
    expect(await screen.findByText('已退出所选会话。')).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: '退出会话 2' }),
      ).not.toBeInTheDocument(),
    );
    const [, request] = fetchMock.mock.calls.find(
      ([input]) => input === '/api/v1/sessions/revoke',
    )!;
    expect(JSON.parse(String(request?.body))).toEqual({
      scope: 'session',
      session_id: otherSession.id,
    });
    expect(request?.headers).toMatchObject({
      'X-CSRF-Token': csrf,
      'X-Studiolo-Request': '1',
    });
    expect(request?.headers).not.toHaveProperty('Origin');
    expect(screen.getByRole('table')).toBeInTheDocument();
  });

  it.each([
    { label: '退出登录', scope: { scope: 'current' } },
    { label: '退出全部会话', scope: { scope: 'all' } },
    {
      label: '退出此浏览器',
      scope: { scope: 'session', session_id: currentSession.id },
    },
  ])(
    '$label 清除工作台和弹层，登录必须再次显式点击',
    async ({ label, scope }) => {
      const fetchMock = mockSessions();
      const user = userEvent.setup();
      renderWorkbench();
      await user.click(
        await screen.findByRole('button', { name: '账户与会话' }),
      );
      await user.click(await screen.findByRole('button', { name: label }));
      expect(
        await screen.findByRole('button', { name: '登录工作台' }),
      ).toBeInTheDocument();
      expect(screen.queryByRole('table')).not.toBeInTheDocument();
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
      const posts = fetchMock.mock.calls.filter(
        ([, init]) => init?.method === 'POST',
      );
      expect(posts).toHaveLength(1);
      expect(JSON.parse(String(posts[0][1]?.body))).toEqual(scope);
    },
  );

  it('列表 401 清除账户弹层及工作台', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/sessions'
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await user.click(await screen.findByRole('button', { name: '账户与会话' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('登录已失效');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });
});
