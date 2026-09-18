/** 策略平面（M4）的封闭前端模型：任何形状不符一律抛错，绝不把缺字段当成「没有扩大项」。 */

export type PolicyKind = 'rules' | 'skills' | 'capability_set';
export type DesiredStatus =
  | 'pending'
  | 'validated'
  | 'rejected'
  | 'activated'
  | 'superseded'
  | 'withdrawn';
export type RevocationLevel = 'instance' | 'type' | 'global';

export type Problem = { path: string; message: string };
export type Expansion = {
  path: string;
  before: string | null;
  after: string | null;
  note: string;
};
export type DiffEntry = {
  op: 'added' | 'removed' | 'changed';
  path: string;
  before: string | null;
  after: string | null;
};
export type Diff = {
  format: string;
  entries: DiffEntry[];
  added: number;
  removed: number;
  changed: number;
  truncated: boolean;
};
export type Report = {
  ok: boolean;
  kind: PolicyKind;
  subject: string;
  retire: boolean;
  problems: Problem[];
  diff: Diff;
  expansions: Expansion[];
  narrowings: Expansion[];
  base_effective_hash: string;
  current_version: number | null;
  body_sha256: string;
};
export type DesiredRecord = {
  id: string;
  kind: PolicyKind;
  subject: string;
  body: string | null;
  body_sha256: string;
  submitter_kind: 'owner' | 'agent';
  agent_type: string | null;
  agent_instance: string | null;
  submitted_at: string;
  status: DesiredStatus;
  report: Report | null;
  validated_at: string | null;
  activated_version: number | null;
};
export type EffectiveEntry = {
  kind: PolicyKind;
  subject: string;
  version: number;
  body_sha256: string;
  activated_at: string;
};
export type RevocationRecord = {
  seq: number;
  level: RevocationLevel;
  target: string;
  revoked_at: string;
  revoked_by: string;
  reason: string;
  released_at: string | null;
  released_by: string | null;
};
export type PolicyStatus = {
  effective_hash: string;
  entries: EffectiveEntry[];
  global_revoked: boolean;
  pending_desired: number;
  revocations: RevocationRecord[];
  l3_available: boolean;
};
export type VersionMeta = {
  version: number;
  kind: PolicyKind;
  subject: string;
  retired: boolean;
  body_sha256: string;
  effective_hash: string;
  previous_effective_hash: string;
  activated_at: string;
  activated_by: string;
  desired_id: string | null;
  rollback_of: number | null;
  expansions: Expansion[];
};
export type VersionRecord = { meta: VersionMeta; body: string | null };
export type Activation = {
  version: number;
  kind: PolicyKind;
  subject: string;
  retired: boolean;
  body_sha256: string;
  effective_hash: string;
  previous_effective_hash: string;
  expansions: Expansion[];
  desired_id: string | null;
  rollback_of: number | null;
  cleared_type_revocation: boolean;
  activated_by: string;
};
export type ActivateOutcome =
  | { kind: 'activated'; activation: Activation; audit_confirmed: boolean }
  | {
      kind: 'released';
      effective_hash: string;
      audit_confirmed: boolean;
    };
export type ActivateRequest =
  | { desired_id: string; expected_effective_hash: string }
  | { rollback_to: number; expected_effective_hash: string }
  | { release_global_revocation: true; expected_effective_hash: string };
export type RevokeRequest = {
  level: RevocationLevel;
  target?: string;
  reason: string;
};
export type DesiredInput = {
  kind: PolicyKind;
  subject: string;
  body: string | null;
};

const KINDS: PolicyKind[] = ['rules', 'skills', 'capability_set'];
const STATUSES: DesiredStatus[] = [
  'pending',
  'validated',
  'rejected',
  'activated',
  'superseded',
  'withdrawn',
];
const LEVELS: RevocationLevel[] = ['instance', 'type', 'global'];

function invalid(): never {
  throw new Error('策略平面响应格式无效，请重新核对。');
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== 'string') invalid();
  return value;
}
function optionalText(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  return text(value);
}
function count(value: unknown): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) invalid();
  return value as number;
}
function optionalCount(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  return count(value);
}
function flag(value: unknown): boolean {
  if (typeof value !== 'boolean') invalid();
  return value;
}
function list<T>(value: unknown, item: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) invalid();
  return value.map(item);
}
function oneOf<T extends string>(value: unknown, allowed: T[]): T {
  if (typeof value !== 'string' || !allowed.includes(value as T)) invalid();
  return value as T;
}
function hash(value: unknown): string {
  const candidate = text(value);
  if (!/^[0-9a-f]{64}$/.test(candidate)) invalid();
  return candidate;
}

export function policyKind(value: unknown): PolicyKind {
  return oneOf(value, KINDS);
}
function problem(value: unknown): Problem {
  const item = record(value);
  return { path: text(item.path), message: text(item.message) };
}
function expansion(value: unknown): Expansion {
  const item = record(value);
  return {
    path: text(item.path),
    before: optionalText(item.before),
    after: optionalText(item.after),
    note: text(item.note),
  };
}
function diff(value: unknown): Diff {
  const item = record(value);
  return {
    format: text(item.format),
    entries: list(item.entries, (entry) => {
      const row = record(entry);
      return {
        op: oneOf(row.op, ['added', 'removed', 'changed'] as const),
        path: text(row.path),
        before: optionalText(row.before),
        after: optionalText(row.after),
      };
    }),
    added: count(item.added),
    removed: count(item.removed),
    changed: count(item.changed),
    truncated: flag(item.truncated),
  };
}
export function report(value: unknown): Report {
  const item = record(value);
  return {
    ok: flag(item.ok),
    kind: policyKind(item.kind),
    subject: text(item.subject),
    retire: flag(item.retire),
    problems: list(item.problems, problem),
    diff: diff(item.diff),
    expansions: list(item.expansions, expansion),
    narrowings: list(item.narrowings, expansion),
    base_effective_hash: hash(item.base_effective_hash),
    current_version: optionalCount(item.current_version),
    body_sha256: hash(item.body_sha256),
  };
}
export function desiredRecord(value: unknown): DesiredRecord {
  const item = record(value);
  return {
    id: text(item.id),
    kind: policyKind(item.kind),
    subject: text(item.subject),
    body: optionalText(item.body),
    body_sha256: text(item.body_sha256),
    submitter_kind: oneOf(item.submitter_kind, ['owner', 'agent'] as const),
    agent_type: optionalText(item.agent_type),
    agent_instance: optionalText(item.agent_instance),
    submitted_at: text(item.submitted_at),
    status: oneOf(item.status, STATUSES),
    report:
      item.report === null || item.report === undefined
        ? null
        : report(item.report),
    validated_at: optionalText(item.validated_at),
    activated_version: optionalCount(item.activated_version),
  };
}
export function desiredList(value: unknown): DesiredRecord[] {
  return list(record(value).desired, desiredRecord);
}
export function desiredEnvelope(value: unknown): DesiredRecord {
  return desiredRecord(record(value).desired);
}
function revocation(value: unknown): RevocationRecord {
  const item = record(value);
  return {
    seq: count(item.seq),
    level: oneOf(item.level, LEVELS),
    target: text(item.target),
    revoked_at: text(item.revoked_at),
    revoked_by: text(item.revoked_by),
    reason: text(item.reason),
    released_at: optionalText(item.released_at),
    released_by: optionalText(item.released_by),
  };
}
export function policyStatus(value: unknown): PolicyStatus {
  const item = record(value);
  return {
    effective_hash: hash(item.effective_hash),
    entries: list(item.entries, (entry) => {
      const row = record(entry);
      return {
        kind: policyKind(row.kind),
        subject: text(row.subject),
        version: count(row.version),
        body_sha256: hash(row.body_sha256),
        activated_at: text(row.activated_at),
      };
    }),
    global_revoked: flag(item.global_revoked),
    pending_desired: count(item.pending_desired),
    revocations: list(item.revocations, revocation),
    l3_available: flag(item.l3_available),
  };
}
function versionMeta(value: unknown): VersionMeta {
  const item = record(value);
  return {
    version: count(item.version),
    kind: policyKind(item.kind),
    subject: text(item.subject),
    retired: flag(item.retired),
    body_sha256: hash(item.body_sha256),
    effective_hash: hash(item.effective_hash),
    previous_effective_hash: hash(item.previous_effective_hash),
    activated_at: text(item.activated_at),
    activated_by: text(item.activated_by),
    desired_id: optionalText(item.desired_id),
    rollback_of: optionalCount(item.rollback_of),
    expansions: list(item.expansions, expansion),
  };
}
export function versionList(value: unknown): VersionMeta[] {
  return list(record(value).versions, versionMeta);
}
export function versionRecord(value: unknown): VersionRecord {
  const item = record(record(value).version);
  return { meta: versionMeta(item.meta), body: optionalText(item.body) };
}
function activation(value: unknown): Activation {
  const item = record(value);
  return {
    version: count(item.version),
    kind: policyKind(item.kind),
    subject: text(item.subject),
    retired: flag(item.retired),
    body_sha256: hash(item.body_sha256),
    effective_hash: hash(item.effective_hash),
    previous_effective_hash: hash(item.previous_effective_hash),
    expansions: list(item.expansions, expansion),
    desired_id: optionalText(item.desired_id),
    rollback_of: optionalCount(item.rollback_of),
    cleared_type_revocation: flag(item.cleared_type_revocation),
    activated_by: text(item.activated_by),
  };
}
export function activateOutcome(value: unknown): ActivateOutcome {
  const item = record(value);
  if ('activation' in item) {
    return {
      kind: 'activated',
      activation: activation(item.activation),
      audit_confirmed: flag(item.audit_confirmed),
    };
  }
  if (item.released !== true) invalid();
  return {
    kind: 'released',
    effective_hash: hash(item.effective_hash),
    audit_confirmed: flag(item.audit_confirmed),
  };
}
export function revokeOutcome(value: unknown): {
  created: boolean;
  level: RevocationLevel;
  target: string;
} {
  const item = record(value);
  return {
    created: flag(item.created),
    level: oneOf(item.level, LEVELS),
    target: text(item.target),
  };
}

export const KIND_LABEL: Record<PolicyKind, string> = {
  rules: '规则母本',
  skills: 'skill',
  capability_set: '能力集',
};
export const STATUS_LABEL: Record<DesiredStatus, string> = {
  pending: '待复核（AI 提案）',
  validated: '已校验，可发布',
  rejected: '校验未通过',
  activated: '已发布',
  superseded: '已被取代',
  withdrawn: '已撤回',
};
export const LEVEL_LABEL: Record<RevocationLevel, string> = {
  instance: '实例',
  type: '类型',
  global: '全局',
};

/** 界面里只显示哈希前 12 位；完整值在悬停标题里。 */
export function shortHash(value: string): string {
  return value.slice(0, 12);
}
