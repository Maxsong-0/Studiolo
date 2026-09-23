import { useState } from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ComponentProps } from 'react';
import type { Freshness } from '../freshness';
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

type Props = ComponentProps<typeof StatusCapsule>;

function Harness({
  initialOpen = false,
  ...overrides
}: Partial<Omit<Props, 'open' | 'onOpenChange'>> & { initialOpen?: boolean }) {
  const [open, setOpen] = useState(initialOpen);
  return (
    <>
      <button type="button">页面上别的按钮</button>
      <button type="button" onClick={() => setOpen(true)}>
        从别处打开
      </button>
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
      <button type="button">顶栏里后面的按钮</button>
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
