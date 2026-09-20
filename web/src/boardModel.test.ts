import { describe, expect, it } from 'vitest';
import { boardItems, boardLease, boardTree } from './boardModel';

const project = '项目/Studiolo';
const hash = 'a'.repeat(64);
const now = Date.UTC(2026, 8, 20, 10);
const id = (n: number) => String(n).padStart(26, '0');
function view(
  n = 1,
  status = 'running',
  parent: string | null = null,
  depth = 0,
) {
  const active = [
    'claimed',
    'running',
    'cancel_requested',
    'committing',
  ].includes(status);
  const attempted =
    active || !['draft', 'queued', 'needs_reconfirm'].includes(status);
  const attemptId = `A-${id(n + 100)}`;
  return {
    task: {
      task_id: id(n),
      spec: {
        role: 'builder',
        adapter: 'consumer',
        input_paths: [],
        allow_web: [],
        allow_db: [],
        retry_policy: { max_attempts: 1, backoff_seconds: 0, retry_on: [] },
        timeout: 600,
        project,
        parent,
        title: `工作项 ${n}`,
        instruction: null,
      },
      status,
      active_attempt: active ? attemptId : null,
      last_attempt: attempted && !active ? attemptId : null,
      input_snapshot: attempted ? hash : null,
    },
    board: {
      task_id: id(n),
      definition: {
        acceptance_criteria: ['契约通过'],
        files_owned: ['项目/Studiolo/03-记忆'],
        max_children: 8,
        policy_hash: hash,
        parent,
        artifact_target: null,
      },
      depth,
      created_by_instance: 'creator',
      handoff: null,
      receipt: null as unknown,
    },
    candidate_stages: [],
    candidate_receipt: null,
    uses_old_policy: false,
    holder: attempted ? 'consumer-instance' : null,
    progress: attempted
      ? { tool_calls: 7, edits: 2, last_progress_ms: now - 500 }
      : null,
    attempt: attempted
      ? {
          attempt_id: attemptId,
          task_id: id(n),
          fence: 42,
          phase: active ? status : 'completed',
          worker_id: 'consumer-instance',
          snapshot_id: hash,
          lease_until_ms: now + 60_000,
          runner_ref_json: null,
          terminal: null,
        }
      : null,
  };
}
function returned() {
  return {
    status: 'completed',
    artifacts: [{ path: 'sources/answer.md', revision: hash, sha256: hash }],
    conflicts: [],
    unlanded: [],
    acceptance: [{ criterion: '契约通过', result: 'pass', evidence: [] }],
    remaining: [],
    elapsed_ms: 1400,
    tool_calls: 7,
    learned: [],
    closeout: {
      learned: [],
      adr_required: false,
      rule_change_required: true,
      destinations: ['项目/Studiolo/03-记忆'],
    },
  };
}

describe('BoardView decoder', () => {
  it('preserves the exact authenticated holder, attempt, fence, lease and pinned-policy evidence', () => {
    const row = view();
    row.uses_old_policy = true;
    const [item] = boardItems([row], project);
    expect(item.attempt).toEqual({
      id: `A-${id(101)}`,
      fence: 42,
      phase: 'running',
      worker: 'consumer-instance',
      leaseUntilMs: now + 60_000,
      active: true,
    });
    expect(item.progress).toEqual({
      tools: 7,
      edits: 2,
      lastProgressMs: now - 500,
    });
    expect(item).toMatchObject({
      holder: 'consumer-instance',
      oldPolicy: true,
      policyHash: hash,
      project,
    });
    expect(boardLease(item, now + 59_999)).toBe('current');
    expect(boardLease(item, now + 60_000)).toBe('expired');
  });

  it('keeps a terminal attempt as history even when its former lease is in the future', () => {
    const [item] = boardItems([view(1, 'succeeded')]);
    expect(item.holder).toBe('consumer-instance');
    expect(item.attempt?.active).toBe(false);
    expect(boardLease(item, now)).toBe('ended');
  });

  it('does not grant a current lease to cancellation or a committing reservation', () => {
    expect(boardLease(boardItems([view(1, 'cancel_requested')])[0], now)).toBe(
      'revoked',
    );
    expect(boardLease(boardItems([view(1, 'committing')])[0], now)).toBe(
      'settling',
    );
    expect(boardLease(boardItems([view(1, 'queued')])[0], now)).toBe('none');
  });

  it('builds a real parent-child tree independently of response order', () => {
    const rows = [
      view(2, 'running', id(1), 1),
      view(1, 'queued'),
      view(3, 'draft', id(2), 2),
    ];
    const tree = boardTree(boardItems(rows));
    expect(tree.map((node) => node.item.id)).toEqual([id(1)]);
    expect(tree[0].children[0].item.id).toBe(id(2));
    expect(tree[0].children[0].children[0].item.id).toBe(id(3));
  });

  it('preserves every receipt assessment, pointer revision and closing decision', () => {
    const row = view(1, 'succeeded');
    row.board.receipt = returned();
    expect(boardItems([row])[0].receipt).toMatchObject({
      elapsedMs: 1400,
      toolCalls: 7,
      artifacts: [{ path: 'sources/answer.md', revision: hash, sha256: hash }],
      acceptance: [{ criterion: '契约通过', result: 'pass', evidence: [] }],
      closeout: { adrRequired: false, ruleChangeRequired: true },
    });
  });

  it('accepts an explicitly empty board but never an absent or wrapped board', () => {
    expect(boardItems([])).toEqual([]);
    for (const value of [null, undefined, {}, { items: [] }, '[]'])
      expect(() => boardItems(value)).toThrow();
  });

  const corruptions: [string, (row: ReturnType<typeof view>) => void][] = [
    [
      'unknown BoardView field',
      (row) => {
        Object.assign(row, { approved: true });
      },
    ],
    [
      'missing attempt field',
      (row) => {
        Reflect.deleteProperty(row, 'attempt');
      },
    ],
    [
      'unknown status',
      (row) => {
        row.task.status = 'automatically_approved';
      },
    ],
    [
      'missing lease',
      (row) => {
        Reflect.deleteProperty(row.attempt!, 'lease_until_ms');
      },
    ],
    [
      'wrong task identity',
      (row) => {
        row.board.task_id = id(2);
      },
    ],
    [
      'wrong attempt task',
      (row) => {
        row.attempt!.task_id = id(2);
      },
    ],
    [
      'wrong active attempt',
      (row) => {
        row.task.active_attempt = `A-${id(99)}`;
      },
    ],
    [
      'wrong holder',
      (row) => {
        row.holder = 'another-instance';
      },
    ],
    [
      'zero fence',
      (row) => {
        row.attempt!.fence = 0;
      },
    ],
    [
      'unsafe fence precision',
      (row) => {
        row.attempt!.fence = Number.MAX_SAFE_INTEGER + 1;
      },
    ],
    [
      'fractional lease',
      (row) => {
        row.attempt!.lease_until_ms = now + 0.25;
      },
    ],
    [
      'negative lease',
      (row) => {
        row.attempt!.lease_until_ms = -1;
      },
    ],
    [
      'invalid date range',
      (row) => {
        row.attempt!.lease_until_ms = 8_640_000_000_000_001;
      },
    ],
    [
      'missing active attempt',
      (row) => {
        row.task.active_attempt = null;
      },
    ],
    [
      'unknown attempt phase',
      (row) => {
        row.attempt!.phase = 'approved';
      },
    ],
    [
      'mismatched live phase',
      (row) => {
        row.attempt!.phase = 'cancel_requested';
      },
    ],
    [
      'missing old-policy marker',
      (row) => {
        Reflect.deleteProperty(row, 'uses_old_policy');
      },
    ],
    [
      'nonboolean old-policy marker',
      (row) => {
        Object.assign(row, { uses_old_policy: 'false' });
      },
    ],
    [
      'malformed policy hash',
      (row) => {
        row.board.definition.policy_hash = 'latest';
      },
    ],
    [
      'no acceptance criteria',
      (row) => {
        row.board.definition.acceptance_criteria = [];
      },
    ],
    [
      'duplicate criteria',
      (row) => {
        row.board.definition.acceptance_criteria.push('契约通过');
      },
    ],
    [
      'path traversal',
      (row) => {
        row.board.definition.files_owned = ['项目/../秘密'];
      },
    ],
    [
      'unknown definition field',
      (row) => {
        Object.assign(row.board.definition, { body: 'untrusted' });
      },
    ],
    [
      'overflow depth',
      (row) => {
        row.board.depth = 4;
      },
    ],
    [
      'overflow fanout',
      (row) => {
        row.board.definition.max_children = 9;
      },
    ],
    [
      'unknown receipt outcome',
      (row) => {
        row.board.receipt = { ...returned(), status: 'approved' };
      },
    ],
  ];
  it.each(corruptions)('rejects %s', (_name, corrupt) => {
    const row = view();
    corrupt(row);
    expect(() => boardItems([row])).toThrow(/不可确认/);
  });

  it('rejects duplicate identities, missing parents, cycles, depth and project mismatch', () => {
    const child = view(2, 'running', id(1), 1);
    expect(() => boardItems([view(), view()])).toThrow();
    expect(() => boardItems([child])).toThrow();
    expect(() => boardItems([view(1, 'queued', id(2), 1), child])).toThrow();
    child.board.depth = 2;
    expect(() => boardItems([view(1, 'queued'), child])).toThrow();
    child.board.depth = 1;
    child.task.spec.project = '项目/另一项目';
    expect(() => boardItems([view(1, 'queued'), child])).toThrow();
    expect(() => boardItems([view()], '项目/另一项目')).toThrow();
  });

  it('rejects incomplete or contradictory receipts instead of presenting completed work', () => {
    for (const raw of [
      { ...returned(), acceptance: [] },
      {
        ...returned(),
        acceptance: [{ criterion: '另一标准', result: 'pass', evidence: [] }],
      },
      {
        ...returned(),
        acceptance: [
          { criterion: '契约通过', result: 'unverified', evidence: [] },
        ],
      },
      { ...returned(), conflicts: ['sources/answer.md'] },
      { ...returned(), remaining: ['还须复测'] },
      {
        ...returned(),
        artifacts: [
          { path: 'sources/answer.md', revision: hash, sha256: 'wrong' },
        ],
      },
      { ...returned(), closeout: { learned: [], destinations: [] } },
    ]) {
      const row = view(1, 'succeeded');
      row.board.receipt = raw;
      expect(() => boardItems([row])).toThrow();
    }
  });
});

it('validates human instance chains independently from task parentage', () => {
  const row = {
    ...view(),
    instance_lineages: [
      [
        { id: 'root-instance', agent_type: 'codex' },
        { id: 'consumer-instance', agent_type: 'codex' },
      ],
    ],
  };
  expect(
    boardItems([row], project)[0].instanceLineages[0].map((n) => n.id),
  ).toEqual(['root-instance', 'consumer-instance']);
  row.instance_lineages[0][1].agent_type = 'claude';
  expect(() => boardItems([row], project)).toThrow();
  row.instance_lineages[0][1] = { id: 'root-instance', agent_type: 'codex' };
  expect(() => boardItems([row], project)).toThrow();
});
