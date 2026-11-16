import { revocation, type RevocationRecord } from '../policyModel';
export const DENIAL_LABEL = {
  global_revoked: '全局已撤销接入',
  type_revoked: '类型已撤销接入',
  instance_revoked: '实例已撤销接入',
  ancestor_revoked: '祖先实例已撤销接入',
  identity_type_revoked: '类型凭据已吊销',
  no_capability_set: '尚无生效能力集',
} as const;
export type AgentDenial = {
  reason: keyof typeof DENIAL_LABEL;
  by: string | null;
};
export type AgentType = {
  name: string;
  issued_at: string;
  identity_revoked_at: string | null;
  capability_version: number | null;
  policy_revocation: RevocationRecord | null;
};
export type AgentInstance = {
  id: string;
  agent_type: string;
  parent: string | null;
  registered_at: string;
  last_seen_at: string;
  denial: AgentDenial | null;
  policy_revocation: RevocationRecord | null;
};
export type AgentsSnapshot = {
  generated_at: string;
  effective_hash: string;
  global_revoked: boolean;
  children_enabled: boolean;
  types: AgentType[];
  instances: AgentInstance[];
  instances_truncated: boolean;
};
function invalid(): never {
  throw new Error('接入方列表响应格式无效，请重新读取。');
}
function record(value: unknown, fields: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>,
    keys = fields.split(' ');
  if (
    Object.keys(row).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(row, key))
  )
    return invalid();
  return row;
}
function text(value: unknown): string {
  if (typeof value !== 'string' || !value || value.length > 4096)
    return invalid();
  return value;
}
function timestamp(value: unknown): string {
  const time = text(value);
  if (!Number.isFinite(Date.parse(time))) return invalid();
  return time;
}
function flag(value: unknown): boolean {
  if (typeof value !== 'boolean') return invalid();
  return value;
}
function nullable<T>(value: unknown, parse: (v: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function list<T>(value: unknown, parse: (v: unknown) => T): T[] {
  if (!Array.isArray(value)) return invalid();
  return value.map(parse);
}
function revoked(value: unknown): RevocationRecord {
  record(
    value,
    'seq level target revoked_at revoked_by reason released_at released_by',
  );
  const result = revocation(value);
  if (result.released_at !== null || result.released_by !== null)
    return invalid();
  return result;
}
function typeRow(value: unknown): AgentType {
  const row = record(
    value,
    'name issued_at identity_revoked_at capability_version policy_revocation',
  );
  return {
    name: text(row.name),
    issued_at: timestamp(row.issued_at),
    identity_revoked_at: nullable(row.identity_revoked_at, timestamp),
    capability_version: nullable(row.capability_version, (v) => {
      if (!Number.isSafeInteger(v) || Number(v) < 1) return invalid();
      return v as number;
    }),
    policy_revocation: nullable(row.policy_revocation, revoked),
  };
}
function instanceRow(value: unknown): AgentInstance {
  const row = record(
    value,
    'id agent_type parent registered_at last_seen_at denial policy_revocation',
  );
  const denial = nullable(row.denial, (value): AgentDenial => {
    const d = record(value, 'reason by');
    if (typeof d.reason !== 'string' || !Object.hasOwn(DENIAL_LABEL, d.reason))
      return invalid();
    const by = nullable(d.by, text);
    if ((d.reason === 'ancestor_revoked') !== (by !== null)) return invalid();
    return { reason: d.reason as AgentDenial['reason'], by };
  });
  return {
    id: text(row.id),
    agent_type: text(row.agent_type),
    parent: nullable(row.parent, text),
    registered_at: timestamp(row.registered_at),
    last_seen_at: timestamp(row.last_seen_at),
    denial,
    policy_revocation: nullable(row.policy_revocation, revoked),
  };
}
export function parseAgents(value: unknown): AgentsSnapshot {
  const row = record(
    value,
    'generated_at effective_hash global_revoked children_enabled types instances instances_truncated',
  );
  const effective_hash = text(row.effective_hash);
  if (!/^[a-f0-9]{64}$/.test(effective_hash)) return invalid();
  const types = list(row.types, typeRow),
    instances = list(row.instances, instanceRow);
  const children_enabled = flag(row.children_enabled);
  if (
    instances.length > 500 ||
    new Set(types.map((t) => t.name)).size !== types.length ||
    new Set(instances.map((i) => i.id)).size !== instances.length
  )
    return invalid();
  for (const t of types)
    if (
      t.policy_revocation &&
      (t.policy_revocation.level !== 'type' ||
        t.policy_revocation.target !== t.name)
    )
      return invalid();
  for (const i of instances) {
    if (
      !types.some((t) => t.name === i.agent_type) ||
      i.parent === i.id ||
      (!children_enabled && i.parent !== null) ||
      (i.policy_revocation &&
        (i.policy_revocation.level !== 'instance' ||
          i.policy_revocation.target !== i.id))
    )
      return invalid();
  }
  return {
    generated_at: timestamp(row.generated_at),
    effective_hash,
    global_revoked: flag(row.global_revoked),
    children_enabled,
    types,
    instances,
    instances_truncated: flag(row.instances_truncated),
  };
}
/** 按服务端快照时间过滤，不把手机时钟偏差当成实例静默；深链接指向的实例始终保留。 */
export function visibleInstances(
  data: AgentsSnapshot,
  recent: boolean,
  selected: string | null,
): AgentInstance[] {
  const since = Date.parse(data.generated_at) - 24 * 60 * 60 * 1000;
  return data.instances.filter(
    (row) =>
      !recent || row.id === selected || Date.parse(row.last_seen_at) >= since,
  );
}
export function relativeCall(value: string, generatedAt: string): string {
  const seconds = Math.max(
    0,
    (Date.parse(generatedAt) - Date.parse(value)) / 1000,
  );
  if (seconds < 60) return '刚刚';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`;
  return `${Math.floor(seconds / 86400)} 天前`;
}
