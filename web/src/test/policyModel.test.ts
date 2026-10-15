import { describe, expect, it } from 'vitest';
import {
  activateOutcome,
  desiredEnvelope,
  desiredList,
  policyStatus,
  report,
  supersededBy,
  versionList,
} from '../policyModel';

const HASH = 'a'.repeat(64);

const okReport = {
  ok: true,
  kind: 'capability_set',
  subject: 'claude',
  retire: false,
  problems: [],
  diff: {
    format: 'json',
    entries: [
      {
        op: 'added',
        path: 'read.project_whitelist[0]',
        before: null,
        after: '"项目/Studiolo"',
      },
    ],
    added: 1,
    removed: 0,
    changed: 0,
    truncated: false,
  },
  expansions: [
    {
      path: 'read.project_whitelist',
      before: null,
      after: '项目/Studiolo',
      note: '新类型首次获得读取范围',
    },
  ],
  narrowings: [],
  base_effective_hash: HASH,
  current_version: null,
  body_sha256: HASH,
};

describe('策略平面模型', () => {
  it('解析状态、desired、版本与发布结果，并拒绝形状不符的响应', () => {
    const status = policyStatus({
      effective_hash: HASH,
      entries: [
        {
          kind: 'rules',
          subject: '通用工作规则.md',
          version: 3,
          body_sha256: HASH,
          activated_at: '2026-09-17T00:00:00Z',
        },
      ],
      global_revoked: false,
      pending_desired: 2,
      revocations: [],
      l3_available: true,
    });
    expect(status.entries[0].version).toBe(3);
    expect(report(okReport).expansions).toHaveLength(1);
    const desired = desiredEnvelope({
      desired: {
        id: '01ABCDEFGHJKMNPQRSTVWXYZ00',
        kind: 'capability_set',
        subject: 'claude',
        body: '{}',
        body_sha256: HASH,
        submitter_kind: 'agent',
        agent_type: 'claude',
        agent_instance: '01INSTANCE',
        submitted_at: '2026-09-17T00:00:00Z',
        status: 'pending',
        report: okReport,
        validated_at: null,
        activated_version: null,
      },
    });
    expect(desired.status).toBe('pending');
    expect(desired.report?.ok).toBe(true);
    expect(
      versionList({
        versions: [
          {
            version: 1,
            kind: 'rules',
            subject: 'a.md',
            retired: false,
            body_sha256: HASH,
            effective_hash: HASH,
            previous_effective_hash: HASH,
            activated_at: '2026-09-17T00:00:00Z',
            activated_by: 'owner',
            desired_id: null,
            rollback_of: null,
            expansions: [],
          },
        ],
      }),
    ).toHaveLength(1);
    const released = activateOutcome({
      released: true,
      effective_hash: HASH,
      audit_confirmed: true,
    });
    expect(released.kind).toBe('released');
    for (const bad of [
      null,
      [],
      {
        effective_hash: 'short',
        entries: [],
        global_revoked: false,
        pending_desired: 0,
        revocations: [],
        l3_available: true,
      },
      {
        effective_hash: HASH,
        entries: [],
        global_revoked: 'no',
        pending_desired: 0,
        revocations: [],
        l3_available: true,
      },
    ]) {
      expect(() => policyStatus(bad)).toThrow();
    }
    expect(() => report({ ...okReport, expansions: undefined })).toThrow();
    expect(() => activateOutcome({ released: false })).toThrow();
  });
});

describe('草案列表的先后表（T-181）', () => {
  const OLDER = '01ABCDEFGHJKMNPQRSTVWXYZ01';
  const NEWER = '01ABCDEFGHJKMNPQRSTVWXYZ02';
  const DONE = '01ABCDEFGHJKMNPQRSTVWXYZ03';
  function row(id: string, status: string) {
    return {
      id,
      kind: 'rules',
      subject: 'CLAUDE.md',
      body: '# 草案\n',
      body_sha256: HASH,
      submitter_kind: 'agent',
      agent_type: 'claude',
      agent_instance: '01INSTANCE',
      submitted_at: '2026-10-15T18:00:00Z',
      status,
      report: okReport,
      validated_at: null,
      activated_version: null,
    };
  }
  const rows = [
    row(NEWER, 'pending'),
    row(OLDER, 'pending'),
    row(DONE, 'activated'),
  ];

  it('每份未发布草案带上「比它更新的」草案号，已发布的是空表；发布最新那份会取代较早的', () => {
    const listed = desiredList({
      desired: rows,
      newer_open: { [OLDER]: [NEWER], [NEWER]: [] },
    });
    expect(listed.map((entry) => [entry.id, entry.newer_open])).toEqual([
      [NEWER, []],
      [OLDER, [NEWER]],
      [DONE, []],
    ]);
    expect(supersededBy(listed[0], listed).map((entry) => entry.id)).toEqual([
      OLDER,
    ]);
    expect(supersededBy(listed[1], listed)).toEqual([]);
  });

  it('先后表缺了、少一份、多一份或形状不对都整份报格式错，绝不当成「没有更新的」', () => {
    const bad = [
      { desired: rows },
      { desired: rows, newer_open: { [NEWER]: [] } },
      {
        desired: rows,
        newer_open: { [OLDER]: [NEWER], [NEWER]: [], [DONE]: [] },
      },
      { desired: rows, newer_open: { [OLDER]: NEWER, [NEWER]: [] } },
      { desired: rows, newer_open: { [OLDER]: [1], [NEWER]: [] } },
    ];
    for (const value of bad)
      expect(() => desiredList(value)).toThrow('策略平面响应格式无效');
  });

  it('先后表自相矛盾也整份拒收：指向自己、别的对象、已结束的或不存在的草案，重复，两份都自称最新', () => {
    const FIRST = '01ABCDEFGHJKMNPQRSTVWXYZ0A';
    const OTHER = '01ABCDEFGHJKMNPQRSTVWXYZ0B';
    const three = [
      row(NEWER, 'pending'),
      row(OLDER, 'validated'),
      row(FIRST, 'rejected'),
    ];
    const elsewhere = [
      ...rows,
      { ...row(OTHER, 'pending'), subject: '其他.md' },
    ];
    const bad = [
      { desired: rows, newer_open: { [OLDER]: [NEWER], [NEWER]: [NEWER] } },
      { desired: rows, newer_open: { [OLDER]: [NEWER], [NEWER]: [OLDER] } },
      { desired: rows, newer_open: { [OLDER]: [], [NEWER]: [] } },
      { desired: rows, newer_open: { [OLDER]: [DONE], [NEWER]: [] } },
      {
        desired: rows,
        newer_open: { [OLDER]: ['01ABCDEFGHJKMNPQRSTVWXYZ99'], [NEWER]: [] },
      },
      {
        desired: elsewhere,
        newer_open: { [OLDER]: [OTHER], [NEWER]: [], [OTHER]: [] },
      },
      {
        desired: three,
        newer_open: { [FIRST]: [NEWER, NEWER], [OLDER]: [NEWER], [NEWER]: [] },
      },
      // 每份列的份数都对，顺序却不是入库先后。
      {
        desired: three,
        newer_open: { [FIRST]: [NEWER, OLDER], [OLDER]: [NEWER], [NEWER]: [] },
      },
    ];
    for (const value of bad)
      expect(() => desiredList(value)).toThrow('策略平面响应格式无效');
    // 前后一致的三份照收；不同对象各排各的。
    expect(
      desiredList({
        desired: three,
        newer_open: { [FIRST]: [OLDER, NEWER], [OLDER]: [NEWER], [NEWER]: [] },
      }).map((entry) => entry.newer_open.length),
    ).toEqual([0, 1, 2]);
    expect(
      desiredList({
        desired: elsewhere,
        newer_open: { [OLDER]: [NEWER], [NEWER]: [], [OTHER]: [] },
      }).map((entry) => entry.newer_open),
    ).toEqual([[], [NEWER], [], []]);
  });
});
