import { historyActor, type HistoryActor } from './historyModel';
export interface ChangesThrough {
  sequence: string;
  epoch: string;
  observation: string;
  generation: string;
}
export type ChangeKind =
  | 'created'
  | 'replaced'
  | 'deleted'
  | 'restored'
  | 'renamed'
  | 'batch'
  | 'batch_rollback'
  | 'gap';
export interface ChangeEntry {
  id: string;
  source: 'broker' | 'observation';
  kind: ChangeKind;
  path: string | null;
  sequence: string | null;
  generation: string | null;
  observation: string | null;
  at_ms: string | null;
  actor: HistoryActor | null;
  previous_revision: string | null;
  revision: string | null;
  bytes: string | null;
  counterpart: string | null;
  gap: 'startup' | 'retention' | 'explanation_incomplete' | null;
  unread: boolean | null;
}
export interface ChangesPage {
  vault_uuid: string;
  project: string | null;
  entries: ChangeEntry[];
  next_cursor: string | null;
  snapshot: string;
  through: ChangesThrough;
  history_incomplete: boolean;
  retired: boolean;
  seen_complete: boolean;
}
export interface SeenRequest {
  vault_uuid: string;
  project: string | null;
  snapshot: string;
  request_id: string;
}
const record = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === 'object' && !Array.isArray(v);
const exact = (v: Record<string, unknown>, keys: string[]) =>
  Object.keys(v).length === keys.length &&
  keys.every((k) => Object.hasOwn(v, k));
const uuid = (v: unknown): v is string =>
  typeof v === 'string' &&
  /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(v);
const uint = (v: unknown): v is string =>
  typeof v === 'string' &&
  /^(0|[1-9][0-9]{0,19})$/.test(v) &&
  BigInt(v) <= 18446744073709551615n;
const time = (v: unknown) =>
  v === null ||
  (typeof v === 'string' &&
    /^(0|-?[1-9][0-9]{0,18})$/.test(v) &&
    BigInt(v) >= -9223372036854775808n &&
    BigInt(v) <= 9223372036854775807n);
const revision = (v: unknown) =>
  v === null || (typeof v === 'string' && /^[0-9a-f]{64}$/.test(v));
const path = (v: unknown): v is string =>
  typeof v === 'string' &&
  v.length > 0 &&
  v.length <= 4096 &&
  !/[\\\p{Cc}]/u.test(v) &&
  v.split('/').every((p) => p !== '' && p !== '.' && p !== '..');
export const changeProject = (v: unknown): v is string =>
  path(v) && v.split('/').length === 2 && v.startsWith('项目/');
const nullablePath = (v: unknown) => v === null || path(v);
const kinds: ChangeKind[] = [
  'created',
  'replaced',
  'deleted',
  'restored',
  'renamed',
  'batch',
  'batch_rollback',
  'gap',
];
function invalid(): never {
  throw new Error('变更流响应格式无效，请重新读取。');
}
export function changesThrough(v: unknown): v is ChangesThrough {
  return (
    record(v) &&
    exact(v, ['sequence', 'epoch', 'observation', 'generation']) &&
    uint(v.sequence) &&
    uuid(v.epoch) &&
    uint(v.observation) &&
    uint(v.generation)
  );
}
export function changesPage(
  value: unknown,
  project: string | null,
  before: string | null,
  previous: ChangesPage | null = null,
): ChangesPage {
  if (
    !record(value) ||
    !exact(value, [
      'vault_uuid',
      'project',
      'entries',
      'next_cursor',
      'snapshot',
      'through',
      'history_incomplete',
      'retired',
      'seen_complete',
    ]) ||
    !uuid(value.vault_uuid) ||
    value.project !== project ||
    !Array.isArray(value.entries) ||
    value.entries.length > 50 ||
    !uuid(value.snapshot) ||
    (value.next_cursor !== null &&
      (!uuid(value.next_cursor) ||
        value.next_cursor !== value.snapshot ||
        value.next_cursor === before)) ||
    !changesThrough(value.through) ||
    typeof value.history_incomplete !== 'boolean' ||
    typeof value.retired !== 'boolean' ||
    typeof value.seen_complete !== 'boolean'
  )
    invalid();
  if (project !== null && !changeProject(project)) invalid();
  const upper = value.through;
  if (
    previous &&
    (value.vault_uuid !== previous.vault_uuid ||
      !Object.entries(previous.through).every(
        ([k, v]) => upper[k as keyof ChangesThrough] === v,
      ))
  )
    invalid();
  const ids = new Set<string>();
  let brokerSequence: bigint | null = null;
  let observed: bigint | null = null;
  for (const row of value.entries) {
    if (
      !record(row) ||
      !exact(row, [
        'id',
        'source',
        'kind',
        'path',
        'sequence',
        'generation',
        'observation',
        'at_ms',
        'actor',
        'previous_revision',
        'revision',
        'bytes',
        'counterpart',
        'gap',
        'unread',
      ]) ||
      typeof row.id !== 'string' ||
      row.id.length > 512 ||
      /\p{Cc}/u.test(row.id) ||
      ids.has(row.id) ||
      !kinds.includes(row.kind as ChangeKind) ||
      !nullablePath(row.path) ||
      !nullablePath(row.counterpart) ||
      !time(row.at_ms) ||
      !historyActor(row.actor) ||
      !revision(row.previous_revision) ||
      !revision(row.revision) ||
      (row.bytes !== null && !uint(row.bytes)) ||
      (row.unread !== null && typeof row.unread !== 'boolean')
    )
      invalid();
    if (row.source === 'broker') {
      if (
        !uint(row.sequence) ||
        row.sequence === '0' ||
        BigInt(row.sequence) > BigInt(value.through.sequence) ||
        row.generation !== null ||
        row.observation !== null ||
        row.gap !== null ||
        row.kind === 'gap' ||
        row.path === null ||
        row.bytes === null ||
        !row.id.startsWith(`b:${row.sequence}:`) ||
        (row.kind === 'renamed') !== (row.counterpart !== null)
      )
        invalid();
      if (brokerSequence !== null && BigInt(row.sequence) > brokerSequence)
        invalid();
      brokerSequence = BigInt(row.sequence);
    } else if (row.source === 'observation') {
      if (
        row.sequence !== null ||
        !uint(row.observation) ||
        row.observation === '0' ||
        BigInt(row.observation) > BigInt(value.through.observation) ||
        !uint(row.generation) ||
        BigInt(row.generation) > BigInt(value.through.generation) ||
        row.actor !== null ||
        row.counterpart !== null ||
        row.id !== `o:${value.through.epoch}:${row.observation}`
      )
        invalid();
      if (row.kind === 'gap') {
        if (
          !['startup', 'retention', 'explanation_incomplete'].includes(
            row.gap as string,
          ) ||
          row.path !== null ||
          row.previous_revision !== null ||
          row.revision !== null ||
          row.bytes !== null
        )
          invalid();
      } else if (
        !['created', 'replaced', 'deleted'].includes(row.kind as string) ||
        row.gap !== null ||
        row.path === null
      )
        invalid();
      if (observed !== null && BigInt(row.observation) >= observed) invalid();
      observed = BigInt(row.observation);
    } else invalid();
    if (row.kind === 'deleted' && row.revision !== null) invalid();
    if (
      row.kind === 'created' &&
      (row.previous_revision !== null || row.revision === null)
    )
      invalid();
    if (
      ['replaced', 'restored', 'batch'].includes(row.kind as string) &&
      row.revision === null
    )
      invalid();
    if (
      row.source === 'observation' &&
      row.kind !== 'gap' &&
      (row.kind === 'deleted') !== (row.bytes === null)
    )
      invalid();
    ids.add(row.id);
  }
  return value as unknown as ChangesPage;
}
export function seenReceipt(value: unknown, page: ChangesPage): void {
  if (
    !record(value) ||
    !exact(value, ['status', 'vault_uuid', 'project', 'through']) ||
    value.status !== 'seen' ||
    value.vault_uuid !== page.vault_uuid ||
    value.project !== page.project ||
    !changesThrough(value.through)
  )
    invalid();
  const upper = value.through;
  if (
    !Object.entries(page.through).every(
      ([k, v]) => upper[k as keyof ChangesThrough] === v,
    )
  )
    invalid();
}
export const changeLabels: Record<ChangeKind, string> = {
  created: '新建',
  replaced: '修改',
  deleted: '删除',
  restored: '恢复',
  renamed: '改名',
  batch: '批量写回',
  batch_rollback: '批量回滚',
  gap: '记录缺口',
};
export function changeActor(row: ChangeEntry): string {
  if (row.source === 'observation') return '外部变化 · 作者未知';
  const a = row.actor;
  if (a === null) return '主体未知';
  if (a.kind === 'obsidian') return 'Obsidian';
  if (a.kind === 'human') return `人侧会话 · ${a.subject}`;
  return `AI · ${a.agent_type} · ${a.instance_id}${a.task_id ? ` · 任务 ${a.task_id}` : ''}${a.attempt_id ? ` · 尝试 ${a.attempt_id}` : ''}`;
}
export function changeTime(value: string | null): string {
  if (value === null) return '时间未知';
  const n = Number(value);
  if (!Number.isSafeInteger(n) || Math.abs(n) > 8640000000000000)
    return '时间超出显示范围';
  return new Date(n).toLocaleString();
}
