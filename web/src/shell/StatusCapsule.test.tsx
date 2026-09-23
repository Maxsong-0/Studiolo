import { useState } from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ComponentProps } from 'react';
import type { Freshness } from '../freshness';
import type { SessionInfo } from '../sessionModel';
import {
  SESSION_ACCOUNT_ATTRIBUTE,
  SessionStatusContext,
} from '../sessionStatus';
import type { SessionStatus } from '../sessionStatus';
import { StatusCapsule } from './StatusCapsule';
import type { HealthStatus } from './useHealthStatus';

const fresh: Freshness = {
  stale: false,
  indexed_through: '12',
  committed_through: '12',
  unresolved: false,
  watcher_healthy: true,
  reason: null,
};

const healthy: HealthStatus = {
  kind: 'ready',
  report: {
    overall: 'normal',
    evidenceComplete: true,
    sections: [
      {
        code: 'disk-quota',
        title: '磁盘配额',
        state: 'normal',
        detail: '占用在预警水位以内',
      },
    ],
  },
};

/** 本机时区里的一个整分钟：胶囊按本机时区显示，期望值与时区无关。 */
const signedInAt = new Date(2026, 8, 23, 1, 44).getTime();
const expiresAt = new Date(2026, 8, 23, 13, 44).getTime();

const activeSession: SessionInfo = {
  id: 'v2-00000000000000a7',
  state: 'active',
  created_at_ms: signedInAt,
  last_seen_at_ms: signedInAt,
  absolute_expires_at_ms: expiresAt,
};

/** SessionBoundary 发下的会话读数：缺省是已登录、会话有效、没有在途与安全验证提示。 */
function signedIn(overrides: Partial<SessionStatus> = {}): SessionStatus {
  return {
    phase: 'ready',
    session: activeSession,
    busy: false,
    factorNotice: null,
    ...overrides,
  };
}

type Props = ComponentProps<typeof StatusCapsule>;

/**
 * 胶囊外面默认套一层会话 Provider（真实页面里由 SessionBoundary 提供）；
 * session=null 表示根本没有 Provider。顶栏账户入口排在「后面的按钮」之后，与真实外壳同序。
 */
function Harness({
  initialOpen = false,
  session = signedIn(),
  ...overrides
}: Partial<Omit<Props, 'open' | 'onOpenChange'>> & {
  initialOpen?: boolean;
  session?: SessionStatus | null;
}) {
  const [open, setOpen] = useState(initialOpen);
  const status = (
    <StatusCapsule
      open={open}
      onOpenChange={setOpen}
      snapshot={{
        sourceLabel: '测试只读库',
        generation: 7,
        freshness: fresh,
      }}
      refreshing={false}
      failed={false}
      onVerify={vi.fn()}
      onRefresh={vi.fn()}
      health={healthy}
      diagnostics={[]}
      {...overrides}
    />
  );
  return (
    <>
      <button type="button">页面上别的按钮</button>
      <button type="button" onClick={() => setOpen(true)}>
        从别处打开
      </button>
      {session === null ? (
        status
      ) : (
        <SessionStatusContext.Provider value={session}>
          {status}
        </SessionStatusContext.Provider>
      )}
      <button type="button">顶栏里后面的按钮</button>
      <button
        type="button"
        aria-label="账户与会话"
        {...{ [SESSION_ACCOUNT_ATTRIBUTE]: '' }}
      >
        账户
      </button>
    </>
  );
}

function capsule() {
  return screen.getByRole('button', { name: /^工作台状态/ });
}

describe('顶栏状态胶囊', () => {
  it('全部正常显示实心方块「正常」；出问题显示最严重一项的红色方括号标签', () => {
    const { rerender } = render(<Harness />);
    // 读屏读到的是「工作台状态：正常」；方块与方括号只是图形，对读屏隐藏。
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*正常$/);
    expect(capsule()).toHaveAttribute('data-level', 'normal');
    expect(capsule()).toHaveTextContent('■正常');
    rerender(
      <Harness
        health={{ kind: 'error', message: '断开' }}
        diagnostics={['灵感盒：缺少版本']}
      />,
    );
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*健康未知$/);
    expect(capsule()).toHaveAttribute('data-level', 'problem');
    expect(capsule()).toHaveTextContent('[健康未知]');
    rerender(<Harness refreshing />);
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*核验中$/);
    expect(capsule()).toHaveAttribute('data-level', 'pending');
    expect(capsule()).toHaveTextContent('□核验中');
  });

  it('点开是浮层：来源与代次、新鲜度原文、本机健康原文与数据提示逐项列出', async () => {
    const user = userEvent.setup();
    render(
      <Harness diagnostics={['灵感盒：缺少版本', '副本：两条同名笔记']} />,
    );
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
    expect(capsule()).not.toHaveAttribute('aria-controls');
    await user.click(capsule());
    expect(capsule()).toHaveAttribute('aria-expanded', 'true');
    const popover = document.getElementById(
      capsule().getAttribute('aria-controls')!,
    )!;
    expect(popover).toHaveAccessibleName('工作台状态');
    expect(within(popover).getByText('测试只读库')).toBeInTheDocument();
    expect(within(popover).getByText('代次 7')).toBeInTheDocument();
    expect(
      within(popover).getByRole('region', { name: '内容新鲜度' }),
    ).toHaveTextContent('已追平已知提交');
    expect(
      within(popover).getByRole('region', { name: '本机健康' }),
    ).toHaveTextContent('占用在预警水位以内');
    expect(within(popover).getByText('数据提示（2）')).toBeInTheDocument();
    expect(
      within(popover)
        .getAllByRole('listitem')
        .map((item) => item.textContent),
    ).toEqual(
      expect.arrayContaining(['灵感盒：缺少版本', '副本：两条同名笔记']),
    );
  });

  it('还没有快照时说明来源状态，不显示新鲜度结论；数据提示为空时明说没有', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<Harness snapshot={null} refreshing />);
    await user.click(capsule());
    const popover = () =>
      document.getElementById(capsule().getAttribute('aria-controls')!)!;
    expect(within(popover()).getByText('读取中')).toBeInTheDocument();
    expect(
      screen.queryByRole('region', { name: '内容新鲜度' }),
    ).not.toBeInTheDocument();
    expect(within(popover()).getByText('没有数据提示。')).toBeInTheDocument();
    rerender(<Harness snapshot={null} failed />);
    expect(within(popover()).getByText('未连接')).toBeInTheDocument();
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*无法读取工作台$/);
  });

  it('「刷新快照」与「重新核验」各走各的动作；在途时 aria-disabled 且不再触发', async () => {
    const user = userEvent.setup();
    const onRefresh = vi.fn();
    const onVerify = vi.fn();
    const { rerender } = render(
      <Harness onRefresh={onRefresh} onVerify={onVerify} />,
    );
    await user.click(capsule());
    await user.click(screen.getByRole('button', { name: '刷新快照' }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: '重新核验' }));
    expect(onVerify).toHaveBeenCalledTimes(1);
    rerender(<Harness onRefresh={onRefresh} onVerify={onVerify} refreshing />);
    const refreshing = screen.getByRole('button', { name: '刷新中' });
    expect(refreshing).toHaveAttribute('aria-disabled', 'true');
    expect(refreshing).not.toBeDisabled();
    await user.click(refreshing);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('Esc 收起并把焦点还给胶囊；点浮层里面不收起，点外面收起', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(capsule());
    await user.click(screen.getByText('测试只读库'));
    expect(capsule()).toHaveAttribute('aria-expanded', 'true');
    screen.getByRole('button', { name: '重新核验' }).focus();
    await user.keyboard('{Escape}');
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
    expect(capsule()).toHaveFocus();
    await user.click(capsule());
    await user.click(screen.getByRole('button', { name: '页面上别的按钮' }));
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
    expect(
      screen.queryByRole('region', { name: '本机健康' }),
    ).not.toBeInTheDocument();
  });

  it('键盘把焦点移出胶囊与浮层时收起，浮层内移动焦点不收起', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(capsule());
    await user.tab();
    expect(screen.getByRole('button', { name: '刷新快照' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: '重新核验' })).toHaveFocus();
    // 会话一栏在新鲜度、健康之后，带路按钮是浮层里最后一个可聚焦的控件。
    await user.tab();
    expect(
      screen.getByRole('button', { name: '前往账户与会话' }),
    ).toHaveFocus();
    expect(capsule()).toHaveAttribute('aria-expanded', 'true');
    // 再按 Tab 就离开胶囊与浮层，落到顶栏里后面的按钮（真实外壳里是 ⌘K 入口）。
    await user.tab();
    expect(
      screen.getByRole('button', { name: '顶栏里后面的按钮' }),
    ).toHaveFocus();
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
  });

  it('焦点只是落到浮层里的文字（relatedTarget 为空）不算移出', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(capsule());
    const verify = screen.getByRole('button', { name: '重新核验' });
    verify.focus();
    fireEvent.focusOut(verify, { relatedTarget: null });
    expect(capsule()).toHaveAttribute('aria-expanded', 'true');
  });

  it('从别处打开时把焦点带进浮层，Esc 仍把焦点交回胶囊', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole('button', { name: '从别处打开' }));
    const popover = document.getElementById(
      capsule().getAttribute('aria-controls')!,
    )!;
    expect(popover).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(capsule()).toHaveFocus();
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
  });
});

describe('顶栏状态胶囊：会话一项', () => {
  function popover() {
    return document.getElementById(capsule().getAttribute('aria-controls')!)!;
  }

  it('会话各状态在胶囊上的标签：正常、核对中、安全验证待处理、会话失效', () => {
    const { rerender } = render(<Harness />);
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*正常$/);
    expect(capsule()).toHaveAttribute('data-level', 'normal');
    rerender(
      <Harness session={signedIn({ phase: 'legacy', session: null })} />,
    );
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*正常$/);
    rerender(<Harness session={signedIn({ busy: true })} />);
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*会话核对中$/);
    expect(capsule()).toHaveAttribute('data-level', 'pending');
    expect(capsule()).toHaveTextContent('□会话核对中');
    rerender(
      <Harness
        session={signedIn({
          factorNotice: '二次认证可能已生效，请先核对状态。',
        })}
      />,
    );
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*安全验证待处理$/);
    expect(capsule()).toHaveAttribute('data-level', 'problem');
    expect(capsule()).toHaveTextContent('[安全验证待处理]');
    rerender(
      <Harness
        session={signedIn({ session: { ...activeSession, state: 'revoked' } })}
      />,
    );
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*会话失效$/);
    expect(capsule()).toHaveTextContent('[会话失效]');
  });

  it('没有会话 Provider 时报「会话未知」，浮层写明会话信息不可用', async () => {
    const user = userEvent.setup();
    render(<Harness session={null} />);
    expect(capsule()).toHaveAccessibleName(/^工作台状态：\s*会话未知$/);
    expect(capsule()).toHaveAttribute('data-level', 'problem');
    await user.click(capsule());
    expect(within(popover()).getByText('会话')).toBeInTheDocument();
    expect(within(popover()).getByText('会话信息不可用')).toBeInTheDocument();
    expect(
      within(popover()).queryByRole('button', { name: '前往账户与会话' }),
    ).not.toBeInTheDocument();
  });

  it('浮层的会话一栏：阶段、ID 前 8 位、登录与到期时间（本机时区），在「本机健康」与「数据提示」之间', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(capsule());
    const titles = within(popover())
      .getAllByRole('heading', { level: 3 })
      .map((heading) => heading.textContent);
    expect(titles).toEqual(['新鲜度', '本机健康', '会话', '数据提示']);
    expect(within(popover()).getByText('已登录')).toBeInTheDocument();
    expect(within(popover()).getByText('v2-00000')).toBeInTheDocument();
    // 只露前 8 位，完整 ID 不进页面文字。
    expect(popover()).not.toHaveTextContent(activeSession.id);
    expect(within(popover()).getByText('2026-09-23 01:44')).toHaveAttribute(
      'datetime',
      new Date(signedInAt).toISOString(),
    );
    expect(within(popover()).getByText('2026-09-23 13:44')).toHaveAttribute(
      'datetime',
      new Date(expiresAt).toISOString(),
    );
    expect(within(popover()).queryByRole('status')).not.toBeInTheDocument();
  });

  it('会话在途写「核对中」；本入口不启用会话登录时只说明、不给账户带路', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<Harness session={signedIn({ busy: true })} />);
    await user.click(capsule());
    expect(within(popover()).getByText('核对中')).toBeInTheDocument();
    rerender(
      <Harness session={signedIn({ phase: 'legacy', session: null })} />,
    );
    expect(
      within(popover()).getByText('本入口未启用会话登录'),
    ).toBeInTheDocument();
    expect(within(popover()).queryByText('v2-00000')).not.toBeInTheDocument();
    expect(
      within(popover()).queryByRole('button', { name: '前往账户与会话' }),
    ).not.toBeInTheDocument();
  });

  it('安全验证提示原文进会话一栏（role=status）', async () => {
    const user = userEvent.setup();
    const notice =
      '恢复码可能已经换发，但本次原码无法重读。核对状态不能确认交付；再次生成是独立的新操作，会使之前的恢复码失效。';
    render(<Harness session={signedIn({ factorNotice: notice })} />);
    await user.click(capsule());
    expect(within(popover()).getByRole('status')).toHaveTextContent(notice);
  });

  it('「前往账户与会话」只把焦点交给顶栏账户入口并收起浮层，不另开对话框', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(capsule());
    expect(
      within(popover()).getByText('从顶栏账户入口打开'),
    ).toBeInTheDocument();
    // 读屏聚焦带路按钮时一并读到去向说明。
    expect(
      screen.getByRole('button', { name: '前往账户与会话' }),
    ).toHaveAccessibleDescription('从顶栏账户入口打开');
    await user.click(screen.getByRole('button', { name: '前往账户与会话' }));
    expect(screen.getByRole('button', { name: '账户与会话' })).toHaveFocus();
    expect(capsule()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
