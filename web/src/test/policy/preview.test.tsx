import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import {
  DESIRED_ID,
  HASH,
  desiredBody,
  policyServer,
  statusBody,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

function previewBox() {
  return screen.getByRole('group', { name: /^执行预览/ });
}
function noPreview() {
  return screen.queryByRole('group', { name: /^执行预览/ });
}
/** a 在文档顺序里排在 b 前面（预览框在触发按钮上方）。 */
function precedes(a: Element, b: Element) {
  return Boolean(
    a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING,
  );
}

async function openPanel(options: Parameters<typeof policyServer>[0]) {
  server = policyServer(options);
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  await screen.findByRole('table', { name: '生效中的各项' });
  return { user, server };
}

describe('发布：执行预览先于请求', () => {
  it('点发布只展开红色虚线框「执行预览」，点「确认发布」之前 fetch 与写入都是零次', async () => {
    const { user, server } = await openPanel({ desired: [desiredBody()] });
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    const reads = server.fetchMock.mock.calls.length;
    const trigger = screen.getByRole('button', { name: '发布（需二次认证）' });

    await user.click(trigger);
    const preview = previewBox();
    expect(preview).toHaveClass('policy-preview');
    expect(preview).toHaveAccessibleName('执行预览 发布 能力集 claude');
    expect(preview).toHaveTextContent('当前生效 v2 → 新版本');
    expect(preview).toHaveTextContent(`生效中摘要 ${HASH.slice(0, 12)}`);
    expect(preview).toHaveTextContent('1 项，见上方校验报告');
    expect(preview).toHaveTextContent('不可撤回');
    expect(preview).toHaveTextContent('一次性验证码');
    expect(preview).toHaveFocus();
    expect(precedes(preview, trigger)).toBe(true);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(trigger).toHaveAttribute('aria-controls', preview.id);
    expect(server.fetchMock.mock.calls).toHaveLength(reads);
    expect(server.posts).toEqual([]);

    await user.click(within(preview).getByRole('button', { name: '确认发布' }));
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: { desired_id: DESIRED_ID, expected_effective_hash: HASH },
        },
      ]),
    );
    expect(noPreview()).not.toBeInTheDocument();
    // 被二次认证挡下：就地弹认证窗口（此处没有受控会话，窗口直说不能认证）。
    expect(
      await screen.findByRole('dialog', { name: '二次认证后继续' }),
    ).toBeInTheDocument();
  });

  it('「取消」收起预览、不发任何请求，焦点回到发布按钮', async () => {
    const { user, server } = await openPanel({ desired: [desiredBody()] });
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    const reads = server.fetchMock.mock.calls.length;
    const trigger = screen.getByRole('button', { name: '发布（需二次认证）' });
    await user.click(trigger);

    await user.click(
      within(previewBox()).getByRole('button', { name: '取消' }),
    );
    expect(noPreview()).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(server.fetchMock.mock.calls).toHaveLength(reads);
    expect(server.posts).toEqual([]);
  });

  it('Esc 与再点一次发布按钮同样只收起预览，不发请求', async () => {
    const { user, server } = await openPanel({ desired: [desiredBody()] });
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    const trigger = screen.getByRole('button', { name: '发布（需二次认证）' });

    await user.click(trigger);
    expect(previewBox()).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(noPreview()).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();

    await user.click(trigger);
    expect(previewBox()).toBeInTheDocument();
    await user.click(trigger);
    expect(noPreview()).not.toBeInTheDocument();
    expect(server.posts).toEqual([]);
  });

  it('预览打开时换选另一份草案，发布预览随之作废', async () => {
    const other = '01ABCDEFGHJKMNPQRSTVWXYZ11';
    const { user } = await openPanel({
      desired: [
        desiredBody(),
        desiredBody({ id: other, subject: 'codex', report: null }),
      ],
    });
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    await user.click(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    );
    expect(previewBox()).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '能力集 codex' }));
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    expect(noPreview()).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    ).toHaveAttribute('aria-expanded', 'false');
  });
});

describe('回滚与解除全局撤销：同样先预览', () => {
  it('回滚：预览写明从哪个版本到哪个版本，预览在该行的回滚按钮上方；确认后才发出', async () => {
    const { user, server } = await openPanel({
      versions: [versionBody(2), versionBody(1)],
    });
    const row = screen.getByRole('button', { name: 'v1' }).closest('li');
    if (!row) throw new Error('缺少 v1 行');
    const trigger = within(row).getByRole('button', {
      name: '回滚到此版本（需二次认证）',
    });
    const reads = server.fetchMock.mock.calls.length;

    await user.click(trigger);
    const preview = within(row).getByRole('group', { name: /^执行预览/ });
    expect(preview).toHaveAccessibleName('执行预览 回滚到 v1');
    expect(preview).toHaveTextContent('当前生效 v2 → 新版本，正文等于 v1');
    expect(preview).toHaveTextContent('回滚前拿不到');
    expect(preview).toHaveTextContent('不可撤回');
    expect(preview).toHaveTextContent('一次性验证码');
    expect(precedes(preview, trigger)).toBe(true);
    expect(server.fetchMock.mock.calls).toHaveLength(reads);
    expect(server.posts).toEqual([]);

    await user.click(within(preview).getByRole('button', { name: '确认回滚' }));
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: { rollback_to: 1, expected_effective_hash: HASH },
        },
      ]),
    );
  });

  it('同一时刻只有一个预览：打开回滚预览会收起发布预览', async () => {
    const { user } = await openPanel({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    await user.click(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    );
    expect(previewBox()).toHaveAccessibleName('执行预览 发布 能力集 claude');
    const row = screen.getByRole('button', { name: 'v1' }).closest('li');
    if (!row) throw new Error('缺少 v1 行');
    await user.click(
      within(row).getByRole('button', { name: '回滚到此版本（需二次认证）' }),
    );
    expect(screen.getAllByRole('group', { name: /^执行预览/ })).toHaveLength(1);
    expect(previewBox()).toHaveAccessibleName('执行预览 回滚到 v1');
  });

  it('解除全局撤销：预览说明范围与恢复路径，确认后才按原 hash 发出', async () => {
    const { user, server } = await openPanel({
      status: statusBody({ global_revoked: true }),
    });
    expect(screen.getByRole('alert')).toHaveTextContent(
      '全局撤销生效中：所有 AI 都被拒绝',
    );
    const trigger = screen.getByRole('button', {
      name: '解除全局撤销（需二次认证）',
    });
    await user.click(trigger);
    const preview = previewBox();
    expect(preview).toHaveTextContent('全局：所有 AI 类型与实例');
    expect(preview).toHaveTextContent('实例级与类型级的撤销照旧生效');
    expect(precedes(preview, trigger)).toBe(true);
    expect(server.posts).toEqual([]);

    await user.click(within(preview).getByRole('button', { name: '确认解除' }));
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: {
            release_global_revocation: true,
            expected_effective_hash: HASH,
          },
        },
      ]),
    );
  });
});

describe('撤销接入：提交只打开预览', () => {
  async function fill(user: ReturnType<typeof userEvent.setup>) {
    const form = screen.getByRole('form', { name: '撤销接入' });
    await user.type(
      within(form).getByRole('textbox', { name: '目标' }),
      '01J0INSTANCE0000000000000',
    );
    await user.type(
      within(form).getByRole('textbox', { name: '理由' }),
      '越界',
    );
    return form;
  }

  it('预览写明撤销范围、理由、即刻生效与不需要一次性验证码；确认才提交原请求', async () => {
    const { user, server } = await openPanel({});
    const form = await fill(user);
    const reads = server.fetchMock.mock.calls.length;
    const submit = within(form).getByRole('button', {
      name: '撤销接入（即刻生效）',
    });

    await user.click(submit);
    const preview = within(form).getByRole('group', { name: /^执行预览/ });
    expect(preview).toHaveAccessibleName(
      '执行预览 撤销接入：实例 01J0INSTANCE0000000000000',
    );
    expect(preview).toHaveTextContent('只拒绝这一个实例');
    expect(preview).toHaveTextContent('越界');
    expect(preview).toHaveTextContent('确认后即刻生效');
    expect(preview).toHaveTextContent('不可撤回：实例级撤销不能恢复。');
    expect(preview).toHaveTextContent('撤销接入本身不需要一次性验证码');
    expect(precedes(preview, submit)).toBe(true);
    expect(submit).toHaveAttribute('aria-expanded', 'true');
    expect(submit).toHaveAttribute('aria-controls', preview.id);
    expect(server.fetchMock.mock.calls).toHaveLength(reads);
    expect(server.posts).toEqual([]);

    await user.click(
      within(preview).getByRole('button', { name: '确认撤销接入' }),
    );
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/agents/revoke',
          body: {
            level: 'instance',
            target: '01J0INSTANCE0000000000000',
            reason: '越界',
          },
        },
      ]),
    );
    expect(
      await screen.findByText('已撤销接入：实例 x（即刻生效）。'),
    ).toBeInTheDocument();
  });

  it('预览打开后改动任何字段，预览作废；取消不发请求', async () => {
    const { user, server } = await openPanel({});
    const form = await fill(user);
    await user.click(
      within(form).getByRole('button', { name: '撤销接入（即刻生效）' }),
    );
    expect(previewBox()).toBeInTheDocument();
    await user.type(within(form).getByRole('textbox', { name: '理由' }), '！');
    expect(noPreview()).not.toBeInTheDocument();

    await user.click(
      within(form).getByRole('button', { name: '撤销接入（即刻生效）' }),
    );
    expect(previewBox()).toHaveTextContent('越界！');
    await user.click(
      within(previewBox()).getByRole('button', { name: '取消' }),
    );
    expect(noPreview()).not.toBeInTheDocument();
    expect(server.posts).toEqual([]);
  });

  it('本地校验不过就只报错，不打开预览', async () => {
    const { user, server } = await openPanel({});
    const form = screen.getByRole('form', { name: '撤销接入' });
    await user.click(
      within(form).getByRole('button', { name: '撤销接入（即刻生效）' }),
    );
    expect(within(form).getByRole('alert')).toHaveTextContent(
      '请填写要撤销接入的实例 id 或类型名。',
    );
    expect(noPreview()).not.toBeInTheDocument();
    expect(server.posts).toEqual([]);
  });
});
