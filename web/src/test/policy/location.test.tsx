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
  policyServer,
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
  it('两个带路目标都在：可被脚本聚焦（tabIndex=-1）但不进 Tab 序列，名字是格子标题', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

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

    const versions = target(COMMAND_TARGETS.policyVersions);
    expect(versions).toHaveAttribute('data-command-target', 'policy-versions');
    expect(versions).toHaveAttribute('tabindex', '-1');
    expect(versions).toHaveAccessibleName('版本与回滚 rollback');
    expect(
      within(versions).getByRole('list', { name: '版本列表' }),
    ).toBeInTheDocument();
  });

  it('#view=policy&item=desired：挂载后队列滚进视野并获得焦点，位置原样不改、不报错、不读版本', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&item=desired';
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    const queue = target(COMMAND_TARGETS.policyQueue);
    await waitFor(() => expect(queue).toHaveFocus());
    expect(scrolled()).toContain(queue);
    expect(window.location.hash).toBe('#view=policy&item=desired');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(
      server
        .reads()
        .filter((path) => path.startsWith('/api/v1/policy/versions/')),
    ).toEqual([]);
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
    expect(target(COMMAND_TARGETS.policyQueue)).not.toHaveFocus();
    expect(scrolled()).not.toContain(target(COMMAND_TARGETS.policyQueue));
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
    await screen.findByRole('table', { name: '生效中的各项' });
    const queue = target(COMMAND_TARGETS.policyQueue);
    await waitFor(() => expect(queue).toHaveFocus());

    // 提交一份草案会刷新整份快照；焦点应留在人正在用的表单里。
    const form = screen.getByRole('form', { name: '提交规则草案' });
    await user.type(
      within(form).getByRole('textbox', { name: '对象' }),
      'claude',
    );
    const reads = server.reads().length;
    await user.click(
      within(form).getByRole('button', { name: '提交并校验（不发布）' }),
    );
    expect(await screen.findByRole('status')).toHaveTextContent(
      '已提交并校验通过',
    );
    // 确实刷新过整份快照（三读），焦点仍不在队列上。
    await waitFor(() => expect(server?.reads().length).toBe(reads + 3));
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
