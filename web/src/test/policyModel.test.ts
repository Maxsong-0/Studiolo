import { describe, expect, it } from 'vitest';
import {
  activateOutcome,
  desiredEnvelope,
  policyStatus,
  report,
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
