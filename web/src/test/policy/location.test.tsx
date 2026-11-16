import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import {
  COMMAND_TARGETS,
  focusCommandTarget,
} from '../../commands/focusTarget';
import {
  desiredBody,
  openTab,
  policyServer,
  tab,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

function target(name: string) {
  const element = document.querySelector<HTMLElement>(
    `[data-command-target="${name}"]`,
  );
  if (!element) throw new Error(`缺少带路目标 ${name}`);
  return element;
}

/** setup.ts 把 scrollIntoView 装成 vi.fn：它的 this 就是被滚进视野的元素。 */
function scrolled() {
  return vi.mocked(HTMLElement.prototype.scrollIntoView).mock.contexts;
}

describe('⌘K 带路：待处理草案队列与版本列表', () => {
  it('两个带路目标各在自己的标签里：可被脚本聚焦（tabIndex=-1）但不进 Tab 序列，名字是格子标题', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    // 版本与回滚在「现状」里。
    const versions = target(COMMAND_TARGETS.policyVersions);
    expect(versions).toHaveAttribute('data-command-target', 'policy-versions');
    expect(versions).toHaveAttribute('tabindex', '-1');
    expect(versions).toHaveAccessibleName('版本与回滚 rollback');
    expect(
      within(versions).getByRole('list', { name: '版本列表' }),
    ).toBeInTheDocument();

    // 待处理草案队列在「待处理」里。
    await openTab(user, '待处理');
    const queue = target(COMMAND_TARGETS.policyQueue);
    expect(queue).toHaveAttribute(
      'data-command-target',
      'policy-desired-queue',
    );
    expect(queue).toHaveAttribute('tabindex', '-1');
    expect(queue).toHaveAccessibleName('待处理草案 desired');
    expect(
      within(queue).getByRole('list', { name: '待处理草案' }),
    ).toBeInTheDocument();
  });

  it('#view=policy&item=desired（旧写法）：落在「待处理」，队列滚进视野并获得焦点，位置原样不改、不报错、不把它当版本号去读', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&item=desired';
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });

    expect(tab('待处理')).toHaveAttribute('aria-selected', 'true');
    const queue = target(COMMAND_TARGETS.policyQueue);
    await waitFor(() => expect(queue).toHaveFocus());
    expect(scrolled()).toContain(queue);
    expect(window.location.hash).toBe('#view=policy&item=desired');
    // 没有出错的那一句（选中草案的校验报告里「N 项权限扩大」也是 alert，不算）。
    expect(document.querySelector('.policy-feedback--error')).toBeNull();
    // 读过的版本正文只有能力差异要比的那一版（校验报告写的 v2），没有哪一次是拿 `desired` 当版本号。
    await waitFor(() =>
      expect(
        server
          ?.reads()
          .filter((path) => path.startsWith('/api/v1/policy/versions/')),
      ).toEqual(['/api/v1/policy/versions/2']),
    );
  });

  it('item 是数字时仍是版本号：回到那一版正文，不去动队列（BUG-015/T-098 不变）', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&item=1';
    render(<PolicyBoundary />);

    expect(await screen.findByText('v1 正文')).toBeInTheDocument();
    expect(window.location.hash).toBe('#view=policy&item=1');
    // 留在「现状」：队列那一格根本没有渲染，更谈不上滚动和聚焦。
    expect(tab('现状')).toHaveAttribute('aria-selected', 'true');
    expect(
      document.querySelector(
        `[data-command-target="${COMMAND_TARGETS.policyQueue}"]`,
      ),
    ).toBeNull();
    expect(scrolled()).toEqual([]);
    expect(screen.getByRole('button', { name: 'v1' })).toHaveAttribute(
      'aria-current',
      'true',
    );
  });

  it('刷新列表不会把焦点再拽回队列：只在位置变成 desired 的那一刻聚焦一次', async () => {
    server = policyServer({
      desired: [desiredBody()],
      write: () => ({ desired: desiredBody() }),
    });
    window.location.hash = '#view=policy&item=desired';
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    const queue = target(COMMAND_TARGETS.policyQueue);
    await waitFor(() => expect(queue).toHaveFocus());

    // 重新校验会刷新整份快照；焦点应留在人正在用的那一格，不被拽回队列。
    const again = screen.getByRole('button', { name: '重新校验' });
    const snapshots = () =>
      server?.reads().filter((path) => path === '/api/v1/policy').length;
    const before = snapshots();
    await user.click(again);
    expect(
      await screen.findByText('已重新校验：已校验，可发布。'),
    ).toHaveAttribute('role', 'status');
    // 确实刷新过整份快照，焦点仍不在队列上，位置也没动。
    await waitFor(() => expect(snapshots()).toBe((before ?? 0) + 1));
    await waitFor(() => expect(again).toBeEnabled());
    expect(target(COMMAND_TARGETS.policyQueue)).toBe(queue);
    expect(queue).not.toHaveFocus();
    expect(window.location.hash).toBe('#view=policy&item=desired');
  });

  it('⌘K 的 focusCommandTarget 能把焦点交给版本与回滚格', async () => {
    server = policyServer({ versions: [versionBody(1)] });
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    const cancel = focusCommandTarget(COMMAND_TARGETS.policyVersions);
    await waitFor(() =>
      expect(target(COMMAND_TARGETS.policyVersions)).toHaveFocus(),
    );
    cancel();
  });
});
