import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ListedDesired, PolicyStatus, Report } from '../../policyModel';
import { DraftPanel } from '../../rules/DraftPanel';
import type { PolicyActions } from '../../rules/usePolicyActions';

/**
 * 规则视图的草案面板（T-181）：只照接口给的 newer_open 判先后——不是最新的就挡住发布并写明更新的是哪份，
 * 是最新的就先说会取代几份较早的，执行预览与发布请求都带上这件事。
 */

const HASH = 'a'.repeat(64);
const OLDER = '01JZ00000AAAXKF8AD4CM26CRT';
const NEWEST = '01JZ00000BBBXKF8AD4CM26CRT';

const status: PolicyStatus = {
  effective_hash: HASH,
  entries: [],
  global_revoked: false,
  pending_desired: 2,
  revocations: [],
  l3_available: true,
};

const report: Report = {
  ok: true,
  kind: 'capability_set',
  subject: 'claude',
  retire: false,
  problems: [],
  diff: {
    format: 'json',
    entries: [],
    added: 0,
    removed: 0,
    changed: 0,
    truncated: false,
  },
  expansions: [],
  narrowings: [],
  base_effective_hash: HASH,
  current_version: 3,
  body_sha256: HASH,
};

function draft(overrides: Partial<ListedDesired>): ListedDesired {
  return {
    id: NEWEST,
    kind: 'capability_set',
    subject: 'claude',
    body: '{}',
    body_sha256: HASH,
    submitter_kind: 'owner',
    agent_type: null,
    agent_instance: null,
    submitted_at: '2026-10-15T18:05:21Z',
    status: 'validated',
    report,
    validated_at: '2026-10-15T18:05:21Z',
    activated_version: null,
    newer_open: [],
    ...overrides,
  };
}

function stubActions(): PolicyActions {
  return {
    busy: false,
    notice: null,
    error: null,
    pending: null,
    preview: null,
    submit: vi.fn(),
    amend: vi.fn(),
    validate: vi.fn(),
    withdraw: vi.fn(),
    togglePreview: vi.fn(),
    confirmPreview: vi.fn(),
    closePreview: vi.fn(),
    retryAfterStepUp: vi.fn(),
    closeStepUp: vi.fn(),
  };
}

function panel(record: ListedDesired, drafts: ListedDesired[]) {
  const actions = stubActions();
  render(
    <DraftPanel
      record={record}
      drafts={drafts}
      title="能力集 claude"
      label="能力集 claude"
      origin="access"
      status={status}
      actions={actions}
    />,
  );
  return actions;
}

describe('草案面板的先后（T-181）', () => {
  it('你的草案之后又来了 AI 提案：写明更新的是哪份、去哪撤回，发布按钮不可点', () => {
    const owner = draft({ id: OLDER, newer_open: [NEWEST] });
    const proposal = draft({ id: NEWEST, submitter_kind: 'agent' });
    panel(owner, [proposal, owner]);
    expect(
      screen.getByText(
        `同一对象还有更新的未发布草案（${NEWEST.slice(0, 12)}）：只能发布最新的那一份；要发这份，先在「策略」页的待处理草案里撤回更新的。`,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    ).toBeDisabled();
    expect(screen.queryByText(/较早的未发布草案/)).not.toBeInTheDocument();
  });

  it('最新的草案：先说会取代几份较早的；展开的执行预览与发布请求都带上它们', async () => {
    const newest = draft({});
    const older = draft({
      id: OLDER,
      submitter_kind: 'agent',
      newer_open: [NEWEST],
    });
    const actions = panel(newest, [newest, older]);
    expect(
      screen.getByText(
        '同一对象另有 1 份较早的未发布草案：发布这一份会把它们标为已取代。',
      ),
    ).toBeInTheDocument();
    await userEvent
      .setup()
      .click(screen.getByRole('button', { name: '发布（需二次认证）' }));
    expect(actions.togglePreview).toHaveBeenCalledWith(
      `publish:${NEWEST}`,
      {
        request: { desired_id: NEWEST, expected_effective_hash: HASH },
        label: '能力集 claude',
        action: '发布 能力集 claude',
        origin: 'access',
        supersedes: 1,
      },
      expect.objectContaining({
        facts: expect.arrayContaining([
          { label: '草案', value: NEWEST },
          {
            label: '同时取代',
            value: `同一对象较早的 1 份未发布草案（${OLDER.slice(0, 12)}）：发布后标为已取代，不能再发布`,
          },
        ]),
      }),
    );
  });
});
