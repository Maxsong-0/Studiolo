import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import { COMMAND_TARGETS } from '../../commands/focusTarget';
import { ApiError } from '../../sessionHttp';
import {
  DESIRED_ID,
  HASH,
  desiredBody,
  policyServer,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

/** 换皮不换行为：重新校验、撤回、提交的本地校验、发布成功后清位置，请求与文案语义都照旧。 */

const MOVED = 'c'.repeat(64);

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

function queueCell() {
  const element = document.querySelector<HTMLElement>(
    `[data-command-target="${COMMAND_TARGETS.policyQueue}"]`,
  );
  if (!element) throw new Error('缺少待处理草案格');
  return element;
}

function writes(path: string, body: unknown) {
  if (path === '/api/v1/policy/validate')
    return { desired: desiredBody({ status: 'validated' }) };
  if (path === '/api/v1/policy/withdraw')
    return { desired: desiredBody({ status: 'withdrawn' }) };
  if (path === '/api/v1/policy/desired')
    return {
      desired: desiredBody({
        subject: (body as { subject: string }).subject,
      }),
    };
  if (path === '/api/v1/policy/activate')
    return {
      activation: {
        version: 3,
        kind: 'capability_set',
        subject: 'claude',
        retired: false,
        body_sha256: HASH,
        effective_hash: MOVED,
        previous_effective_hash: HASH,
        expansions: [],
        desired_id: DESIRED_ID,
        rollback_of: null,
        cleared_type_revocation: false,
        activated_by: 'owner',
      },
      audit_confirmed: true,
    };
  throw new ApiError('未知', 404);
}

async function openWithDraft() {
  server = policyServer({
    desired: [desiredBody()],
    versions: [versionBody(2), versionBody(1)],
    write: writes,
  });
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  await screen.findByRole('table', { name: '生效中的各项' });
  await user.click(screen.getByRole('button', { name: '能力集 claude' }));
  return { user, server };
}

describe('待处理草案的动作', () => {
  it('重新校验：直接发出（可逆，不走执行预览），提示落在待处理草案格', async () => {
    const { user, server } = await openWithDraft();
    await user.click(screen.getByRole('button', { name: '重新校验' }));
    await waitFor(() =>
      expect(server.posts).toEqual([
        { path: '/api/v1/policy/validate', body: { desired_id: DESIRED_ID } },
      ]),
    );
    expect(
      await within(queueCell()).findByText('已重新校验：已校验，可发布。'),
    ).toHaveAttribute('role', 'status');
    expect(
      screen.queryByRole('group', { name: /^执行预览/ }),
    ).not.toBeInTheDocument();
  });

  it('撤回：发出撤回请求，提示「已撤回。」，选中的草案与报告收起', async () => {
    const { user, server } = await openWithDraft();
    expect(screen.getByRole('group', { name: '校验报告' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '撤回' }));
    await waitFor(() =>
      expect(server.posts).toEqual([
        { path: '/api/v1/policy/withdraw', body: { desired_id: DESIRED_ID } },
      ]),
    );
    expect(await within(queueCell()).findByText('已撤回。')).toBeVisible();
    expect(
      screen.queryByRole('group', { name: '校验报告' }),
    ).not.toBeInTheDocument();
  });

  it('发布成功：提示新版本与生效中摘要，位置里的版本号作废（item 清掉），版本正文收起', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
      write: writes,
    });
    window.location.hash = '#view=policy&item=2';
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    expect(await screen.findByText('v2 正文')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    await user.click(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    );
    await user.click(screen.getByRole('button', { name: '确认发布' }));

    expect(
      await within(queueCell()).findByText(
        `能力集 claude：已发布为 v3，生效中摘要 ${MOVED.slice(0, 12)}。`,
      ),
    ).toBeInTheDocument();
    expect(server.posts).toEqual([
      {
        path: '/api/v1/policy/activate',
        body: { desired_id: DESIRED_ID, expected_effective_hash: HASH },
      },
    ]);
    expect(window.location.hash).not.toContain('item=');
    expect(screen.queryByText('v2 正文')).not.toBeInTheDocument();
  });
});

describe('提交规则草案的本地校验（不发请求）', () => {
  it('对象为空、正文为空时只报错；勾选「撤销该对象」则以空正文提交', async () => {
    server = policyServer({ write: writes });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    const form = await screen.findByRole('form', { name: '提交规则草案' });
    const submit = within(form).getByRole('button', {
      name: '提交并校验（不发布）',
    });

    await user.click(submit);
    expect(within(form).getByRole('alert')).toHaveTextContent(
      '请填写对象（规则文件名、skill 名或 AI 类型名）。',
    );

    await user.type(
      within(form).getByRole('textbox', { name: '对象' }),
      'claude',
    );
    await user.clear(
      within(form).getByRole('textbox', {
        name: '正文（规则为 Markdown，其余为 JSON）',
      }),
    );
    await user.click(submit);
    expect(within(form).getByRole('alert')).toHaveTextContent(
      '请填写正文，或勾选「撤销该对象」。',
    );
    expect(server.posts).toEqual([]);

    await user.click(
      within(form).getByRole('checkbox', { name: '撤销该对象' }),
    );
    await user.click(submit);
    await waitFor(() =>
      expect(server?.posts).toEqual([
        {
          path: '/api/v1/policy/desired',
          body: { kind: 'capability_set', subject: 'claude', body: null },
        },
      ]),
    );
  });
});
