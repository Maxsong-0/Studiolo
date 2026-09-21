import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { App } from '../App';
import { SessionBoundary } from '../SessionBoundary';
import { mockSessions } from './sessionFixtures';

function boundary() {
  return (
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>
  );
}

function heading(name: string) {
  return screen.queryByRole('heading', { level: 1, name });
}

/** 真实的切走再回来：页面隐藏触发 suspend，恢复可见触发 resume→重新核对→整棵工作台重挂。 */
async function leaveAndReturn() {
  for (const state of ['hidden', 'visible'] as const) {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => state,
    });
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
  }
}

describe('离开再回来的位置', () => {
  it('会话仍有效时，重新核对后回到原视图与原项目，而不是回落首页', async () => {
    mockSessions();
    const user = userEvent.setup();
    render(boundary());
    await screen.findByRole('table');

    await user.click(screen.getByRole('button', { name: /协作黑板/ }));
    expect(heading('协作黑板')).toBeInTheDocument();
    await user.selectOptions(
      screen.getByRole('combobox', { name: '项目' }),
      '项目/资料夹',
    );
    expect(window.location.hash).toContain('view=board');

    await leaveAndReturn();

    // 重新核对期间内容照约定关掉，核对完成后位置原样回来。
    await waitFor(() => expect(heading('协作黑板')).toBeInTheDocument());
    expect(heading('项目总览')).not.toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: '项目' })).toHaveValue(
      '项目/资料夹',
    );
  });

  it('会话真的失效时走登录，登录后仍回到离开处', async () => {
    let signedOut = false;
    const fetchMock = mockSessions();
    const live = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) => {
      if (signedOut && input === '/api/v1/session')
        return new Response('{}', { status: 401 });
      if (input === '/api/v1/sessions' && init?.method === 'POST')
        signedOut = false;
      return live(input, init);
    });
    const user = userEvent.setup();
    render(boundary());
    await screen.findByRole('table');
    await user.click(screen.getByRole('button', { name: /文档/ }));
    await user.click(screen.getByRole('button', { name: '文件树' }));
    expect(window.location.hash).toContain('panel=tree');

    signedOut = true;
    await leaveAndReturn();

    const login = await screen.findByRole('button', { name: '登录工作台' });
    expect(heading('文档')).not.toBeInTheDocument();
    await user.click(login);

    await waitFor(() => expect(heading('文档')).toBeInTheDocument());
    expect(screen.getByRole('button', { name: '文件树' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('直接打开带位置的链接就落在该位置，损坏的位置回落总览', async () => {
    mockSessions();
    window.location.hash = '#view=policy';
    const first = render(boundary());
    await waitFor(() => expect(heading('策略平面')).toBeInTheDocument());
    first.unmount();

    window.location.hash = '#view=%E4%B8%8D%E5%AD%98%E5%9C%A8';
    render(boundary());
    await waitFor(() => expect(heading('项目总览')).toBeInTheDocument());
  });
});
