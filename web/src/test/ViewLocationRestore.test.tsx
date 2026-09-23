import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { SessionBoundary } from '../SessionBoundary';
import { REVISIT_INTERVAL_MS } from '../sessionClient';
import { mockSessions } from './sessionFixtures';

function boundary() {
  return (
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>
  );
}

function heading(name: string) {
  return screen.queryByRole('heading', { level: 1, name });
}

/**
 * 真实的切走再回来。一天内回来不再核对（ADR-022）；要走到「重新核对→整页重挂」
 * 这条路，先把时钟拨过一天。
 */
async function leaveAndReturn({ afterADay = false } = {}) {
  if (afterADay)
    vi.spyOn(Date, 'now').mockReturnValue(
      Date.now() + REVISIT_INTERVAL_MS + 1000,
    );
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

    await user.click(screen.getByRole('button', { name: '协作' }));
    await user.click(screen.getByRole('tab', { name: '任务与回执' }));
    expect(heading('协作')).toBeInTheDocument();
    await user.selectOptions(
      screen.getByRole('combobox', { name: '项目' }),
      '项目/资料夹',
    );
    expect(window.location.hash).toContain('view=collab');

    // 一天内切回：什么都不发生，位置自然还在。
    await leaveAndReturn();
    expect(heading('协作')).toBeInTheDocument();

    // 超过一天切回：重新核对、整棵重挂，位置从 URL 读回。
    await leaveAndReturn({ afterADay: true });
    await waitFor(() => expect(heading('协作')).toBeInTheDocument());
    expect(heading('驾驶舱')).not.toBeInTheDocument();
    expect(screen.getByRole('tab', { name: '任务与回执' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
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
    await user.click(screen.getByRole('button', { name: '文档' }));
    await user.click(screen.getByRole('button', { name: '文件树' }));
    expect(window.location.hash).toContain('panel=tree');

    signedOut = true;
    await leaveAndReturn({ afterADay: true });

    const login = await screen.findByRole('button', { name: '登录工作台' });
    expect(heading('文档')).not.toBeInTheDocument();
    await user.click(login);

    await waitFor(() => expect(heading('文档')).toBeInTheDocument());
    expect(screen.getByRole('button', { name: '文件树' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('直接打开带位置的链接就落在该位置，损坏的位置回落驾驶舱', async () => {
    mockSessions();
    window.location.hash = '#view=policy';
    const first = render(boundary());
    await waitFor(() => expect(heading('策略')).toBeInTheDocument());
    first.unmount();

    window.location.hash = '#view=%E4%B8%8D%E5%AD%98%E5%9C%A8';
    render(boundary());
    await waitFor(() => expect(heading('驾驶舱')).toBeInTheDocument());
  });

  it('旧七视图的链接照样打开，落到新结构里的对应位置（设计规范第 4 节）', async () => {
    mockSessions();
    const project = encodeURIComponent('项目/资料夹');

    window.location.hash = `#view=board&panel=replay&project=${project}`;
    let view = render(boundary());
    await waitFor(() => expect(heading('协作')).toBeInTheDocument());
    expect(screen.getByRole('tab', { name: '时间线' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('combobox', { name: '项目' })).toHaveValue(
      '项目/资料夹',
    );
    view.unmount();

    window.location.hash = `#view=operations&project=${project}`;
    view = render(boundary());
    await waitFor(() => expect(heading('项目运作')).toBeInTheDocument());
    expect(screen.getByRole('combobox', { name: '项目' })).toHaveValue(
      '项目/资料夹',
    );
    view.unmount();

    window.location.hash = '#view=inbox';
    render(boundary());
    await waitFor(() => expect(heading('驾驶舱')).toBeInTheDocument());
    expect(
      screen.getByRole('region', { name: '待我动作' }),
    ).toBeInTheDocument();
  });
});
