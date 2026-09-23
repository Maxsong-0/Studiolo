import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../../App';
import { SessionBoundary } from '../../SessionBoundary';
import { factorResponse, mockFactors } from '../factorFixtures';
import { fixture, jsonResponse } from '../fixtures';
import {
  currentSession,
  mockSessions,
  sessionResponse,
} from '../sessionFixtures';

/**
 * 会话读数经 SessionBoundary 的 context 进顶栏状态胶囊（T-130 C2）：
 * 胶囊不另发请求、不另开账户对话框，只读 SessionClient 快照里的阶段、会话、在途与安全验证提示。
 */

const fresh = {
  stale: false,
  indexed_through: '12',
  committed_through: '12',
  watcher_healthy: true,
  unresolved: false,
  reason: null,
};

/** 新鲜度与本机健康都正常：胶囊上剩下的只有会话一项。 */
function calm(input: RequestInfo | URL): Response | null {
  const path = String(input);
  if (path.startsWith('/api/workbench'))
    return jsonResponse({ ...fixture, freshness: fresh });
  if (path === '/api/v1/health')
    return jsonResponse({
      overall: 'normal',
      evidence_complete: true,
      sections: [
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'normal',
          detail: '占用在预警水位以内',
        },
      ],
    });
  return null;
}

function renderWorkbench() {
  return render(
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>,
  );
}

function capsule() {
  return screen.getByRole('button', { name: /^工作台状态/ });
}

function popover() {
  return document.getElementById(capsule().getAttribute('aria-controls')!)!;
}

describe('状态胶囊的会话一项（经 SessionBoundary）', () => {
  it('已登录：胶囊报正常，浮层列出阶段、ID 前 8 位与本机时区的登录、到期时间', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(
      async (input, init) => calm(input) ?? sessionResponse(input, init),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await screen.findByRole('table');
    await within(capsule()).findByText('正常');
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*正常$/);
    await user.click(capsule());
    const block = within(popover());
    expect(block.getByText('已登录')).toBeInTheDocument();
    expect(block.getByText(currentSession.id.slice(0, 8))).toBeInTheDocument();
    expect(popover()).not.toHaveTextContent(currentSession.id);
    const times = block
      .getAllByText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/)
      .map((time) => time.getAttribute('datetime'));
    expect(times).toEqual([
      new Date(currentSession.created_at_ms).toISOString(),
      new Date(currentSession.absolute_expires_at_ms).toISOString(),
    ]);
    // 胶囊只读快照：打开浮层不发任何会话请求。
    expect(
      fetchMock.mock.calls.filter(([path]) => path === '/api/v1/sessions'),
    ).toHaveLength(0);
  });

  it('本入口不启用会话登录（local 引导）：这一项算正常，浮层只说明、不给账户带路', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/bootstrap'
        ? jsonResponse({ authentication: 'local' })
        : (calm(input) ?? sessionResponse(input, init)),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await screen.findByRole('table');
    await within(capsule()).findByText('正常');
    await user.click(capsule());
    expect(
      within(popover()).getByText('本入口未启用会话登录'),
    ).toBeInTheDocument();
    expect(
      within(popover()).queryByRole('button', { name: '前往账户与会话' }),
    ).not.toBeInTheDocument();
  });

  it('换发结果不确定、核对登录后：胶囊报「安全验证待处理」，会话一栏给出提示原文', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/recovery-codes'
        ? jsonResponse({}, 503)
        : (calm(input) ?? factorResponse(input, init)),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await user.click(await screen.findByRole('button', { name: '账户与会话' }));
    await user.click(await screen.findByRole('button', { name: '生成恢复码' }));
    await user.click(screen.getByRole('button', { name: '确认生成新恢复码' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('原码无法重读');
    await user.click(screen.getByRole('button', { name: '核对登录状态' }));
    await screen.findByRole('table');
    await within(capsule()).findByText('安全验证待处理');
    expect(capsule()).toHaveAttribute('data-level', 'problem');
    await user.click(capsule());
    expect(within(popover()).getByRole('status')).toHaveTextContent(
      '再次生成是独立的新操作',
    );
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
  });

  it('「前往账户与会话」把焦点交给顶栏账户入口并收起浮层，不另开对话框', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(
      async (input, init) => calm(input) ?? sessionResponse(input, init),
    );
    const user = userEvent.setup();
    renderWorkbench();
    await screen.findByRole('table');
    await user.click(capsule());
    await user.click(screen.getByRole('button', { name: '前往账户与会话' }));
    expect(screen.getByRole('button', { name: '账户与会话' })).toHaveFocus();
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(
      fetchMock.mock.calls.filter(([path]) => path === '/api/v1/sessions'),
    ).toHaveLength(0);
  });
});
