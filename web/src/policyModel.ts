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
/**
 * 草案列表里的一份（T-181）：附「同一对象里比它晚入库的未发布草案号」，按入库先后；最新那份、
 * 已发布、已取代、已撤回的都是空表。发布时服务端按同一张表拦截较早的草案，界面只照它标「最新」「较早」。
 */
export type ListedDesired = DesiredRecord & { newer_open: string[] };
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
/** 还没发布、仍可发布／重新校验／撤回的三种状态（与服务端 pending／validated／rejected 同口径）。 */
const OPEN_STATUSES: readonly DesiredStatus[] = [
  'pending',
  'validated',
  'rejected',
];

export function isOpenStatus(status: DesiredStatus): boolean {
  return OPEN_STATUSES.includes(status);
}

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
/**
 * 草案列表：每份未发布草案都必须在 `newer_open` 里有一张表，别的草案都不能有；缺了就整份报格式错，
 * 绝不当成「没有更新的草案」——那正是较早草案被当成最新发出去的那类静默（BUG-031）。
 * 表还得前后一致（见 `checkNewerOpen`），自相矛盾的同样整份拒收。
 */
export function desiredList(value: unknown): ListedDesired[] {
  const item = record(value);
  const newer = record(item.newer_open);
  const desired = list(item.desired, desiredRecord);
  const open = desired.filter((entry) => isOpenStatus(entry.status));
  if (
    Object.keys(newer).length !== open.length ||
    open.some((entry) => !Object.hasOwn(newer, entry.id))
  )
    invalid();
  const orders = new Map(
    open.map((entry) => [entry.id, list(newer[entry.id], text)]),
  );
  checkNewerOpen(open, orders);
  return desired.map((entry) => ({
    ...entry,
    newer_open: isOpenStatus(entry.status) ? (orders.get(entry.id) ?? []) : [],
  }));
}

/**
 * `newer_open` 必须排出同一对象内唯一的入库先后：一个对象有 n 份未发布草案时，第 k 早的那份
 * 按入库先后列出比它晚的 n-1-k 份，最新那份是空表。指向别的对象或自己、重复、两份都自称最新，
 * 都整份拒收：界面只照这张表挡发布、选编辑底稿，不自己猜。
 */
function checkNewerOpen(
  open: readonly DesiredRecord[],
  orders: ReadonlyMap<string, readonly string[]>,
): void {
  const objects = new Map<string, string[]>();
  for (const entry of open) {
    const key = `${entry.kind}\u0000${entry.subject}`;
    objects.set(key, [...(objects.get(key) ?? []), entry.id]);
  }
  for (const ids of objects.values()) {
    // 每份后面还有几份更新的：同一对象里各不相同，先后才唯一。
    const rank = new Map(ids.map((id) => [id, orders.get(id)?.length ?? -1]));
    if (new Set(rank.values()).size !== ids.length) invalid();
    for (const id of ids) {
      const newer = orders.get(id) ?? [];
      // 第 i 份更新的草案后面还剩 newer.length-1-i 份：只能是同一对象里、按入库先后排好的那几份。
      if (
        newer.some(
          (other, index) => rank.get(other) !== newer.length - 1 - index,
        )
      )
        invalid();
    }
  }
}

/** 发布这一份会一并标为已取代的：同一对象里比它早入库的未发布草案（照接口给的 `newer_open` 判）。 */
export function supersededBy(
  target: ListedDesired,
  desired: readonly ListedDesired[],
): ListedDesired[] {
  return desired.filter((entry) => entry.newer_open.includes(target.id));
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

/** 草案号（ULID）只显示前 12 位：前 10 位是毫秒时间，再带 2 位随机，足以区分同一对象的几份草案。 */
export function shortId(id: string): string {
  return id.slice(0, 12);
}
