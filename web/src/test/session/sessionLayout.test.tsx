import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../../App';
import { SessionBoundary } from '../../SessionBoundary';
import { jsonResponse } from '../fixtures';
import {
  currentSession,
  mockSessions,
  otherSession,
  sessionResponse,
} from '../sessionFixtures';

/**
 * 登录闸门与账户对话框的新视觉（T-130 C2）只换样子：这里钉住换样子后仍在的内容与语义——
 * 闸门的字标、图名与唯一动作；当前登录小表的三格读数；会话记录每行的状态文字与撤销按钮。
 */

function renderWorkbench() {
  return render(
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>,
  );
}

describe('登录闸门', () => {
  it('未登录：字标、图名「进入你的工作台」、说明原文与唯一的登录动作', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/session'
        ? jsonResponse({}, 401)
        : sessionResponse(input, init),
    );
    renderWorkbench();
    const login = await screen.findByRole('button', { name: '登录工作台' });
    const gate = screen.getByRole('region', { name: '进入你的工作台' });
    expect(
      within(gate).getByRole('heading', { level: 1, name: '进入你的工作台' }),
    ).toBeInTheDocument();
    expect(within(gate).getByText('Studiolo')).toBeInTheDocument();
    expect(within(gate).getByRole('status')).toHaveTextContent(
      '登录后即可进入工作台。',
    );
    expect(within(gate).getAllByRole('button')).toEqual([login]);
  });

  it('核对中没有动作按钮，只有一条读屏播报', () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(() => new Promise<Response>(() => {}));
    renderWorkbench();
    const gate = screen.getByRole('region', { name: '核对登录状态' });
    expect(within(gate).getByRole('status')).toHaveTextContent('正在核对');
    expect(within(gate).queryByRole('button')).not.toBeInTheDocument();
  });
});

describe('账户与会话对话框', () => {
  it('当前登录小表：状态、登录于、到期时间三格，时间等宽且带机读时间，不露完整会话 ID', async () => {
    mockSessions();
    const user = userEvent.setup();
    renderWorkbench();
    await user.click(await screen.findByRole('button', { name: '账户与会话' }));
    const current = await screen.findByRole('region', { name: '当前登录' });
    const facts = within(current)
      .getAllByRole('term')
      .map((term) => term.textContent);
    expect(facts).toEqual(['状态', '登录于', '到期时间']);
    expect(within(current).getByText('有效')).toBeInTheDocument();
    const times = within(current)
      .getAllByText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/)
      .map((time) => time.getAttribute('datetime'));
    expect(times).toEqual([
      new Date(currentSession.created_at_ms).toISOString(),
      new Date(currentSession.absolute_expires_at_ms).toISOString(),
    ]);
    expect(document.body.textContent).not.toContain(currentSession.id);
  });

  it('会话记录每行都写着状态文字；已结束的会话没有撤销按钮，有效的才有', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/sessions' && init?.method === 'GET'
        ? jsonResponse({
            sessions: [
              currentSession,
              { ...otherSession, state: 'revoked' },
              {
                ...otherSession,
                id: 'v2-0000000000000003',
                state: 'expired',
              },
            ],
          })
        : sessionResponse(input, init),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await user.click(await screen.findByRole('button', { name: '账户与会话' }));
    const list = await screen.findByRole('list', { name: '会话记录' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows).toHaveLength(3);
    expect(within(rows[0]).getByText('有效')).toBeInTheDocument();
    expect(within(rows[1]).getByText('已退出')).toBeInTheDocument();
    expect(within(rows[2]).getByText('已过期')).toBeInTheDocument();
    expect(
      within(rows[0]).getByRole('button', { name: '退出此浏览器' }),
    ).toBeInTheDocument();
    expect(within(rows[1]).queryByRole('button')).not.toBeInTheDocument();
    expect(within(rows[2]).queryByRole('button')).not.toBeInTheDocument();
    expect(
      within(rows[1]).getAllByText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/),
    ).toHaveLength(2);
  });
});
