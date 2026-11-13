export type ReadOutcome = 'read' | 'denied';
export type ReadAuditQuery = {
  outcome: ReadOutcome;
  agent_type: string | null;
  instance_id: string | null;
  since: string | null;
  until: string | null;
  path_prefix: string | null;
  before: number | null;
};
export type ReadAuditRow = {
  offset: number;
  recorded_at: string;
  agent_type: string | null;
  instance_id: string | null;
  action: string;
  path: string | null;
  revision: string | null;
  reason: string | null;
};
export type ReadAuditPage = {
  generated_at: string;
  outcome: ReadOutcome;
  head_offset: number;
  scanned: number;
  next_before: number | null;
  records: ReadAuditRow[];
};
function invalid(): never {
  throw new Error('审计查询响应格式无效，请重新查询。');
}
function object(value: unknown, names: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>,
    keys = names.split(' ');
  if (
    Object.keys(row).length !== keys.length ||
    keys.some((k) => !Object.hasOwn(row, k))
  )
    return invalid();
  return row;
}
function text(value: unknown, max = 4096): string {
  if (
    typeof value !== 'string' ||
    !value ||
    value.length > max ||
    /\p{Cc}/u.test(value)
  )
    return invalid();
  return value;
}
function time(value: unknown): string {
  const result = text(value, 64);
  if (
    !/^\d{4}-\d{2}-\d{2}T/.test(result) ||
    !Number.isFinite(Date.parse(result))
  )
    return invalid();
  return result;
}
function integer(
  value: unknown,
  min: number,
  max = Number.MAX_SAFE_INTEGER,
): number {
  if (
    !Number.isSafeInteger(value) ||
    Number(value) < min ||
    Number(value) > max
  )
    return invalid();
  return Number(value);
}
function nullable<T>(value: unknown, decode: (value: unknown) => T): T | null {
  return value === null ? null : decode(value);
}
export function parseReadAudit(
  value: unknown,
  expected: ReadOutcome,
  before: number | null = null,
): ReadAuditPage {
  const row = object(
    value,
    'generated_at outcome head_offset scanned next_before records',
  );
  if (row.outcome !== expected || !Array.isArray(row.records)) return invalid();
  const head_offset = integer(row.head_offset, 0, Number.MAX_SAFE_INTEGER - 1),
    scanned = integer(row.scanned, 0, 256),
    next_before = nullable(row.next_before, (v) => integer(v, 2));
  const records = row.records.map((value) => {
    const r = object(
      value,
      'offset recorded_at agent_type instance_id action path revision reason',
    );
    const result: ReadAuditRow = {
      offset: integer(r.offset, 1, head_offset),
      recorded_at: time(r.recorded_at),
      agent_type: nullable(r.agent_type, (v) => text(v, 32)),
      instance_id: nullable(r.instance_id, (v) => text(v, 64)),
      action: text(r.action, 128),
      path: nullable(r.path, text),
      revision: nullable(r.revision, (v) => text(v, 128)),
      reason: nullable(r.reason, (v) => text(v, 128)),
    };
    if (
      expected === 'read'
        ? result.path === null ||
          result.revision === null ||
          result.reason !== null
        : result.path !== null ||
          result.revision !== null ||
          result.reason === null
    )
      return invalid();
    return result;
  });
  if (
    records.length > scanned ||
    (before !== null &&
      (records.some((r) => r.offset >= before) ||
        (next_before !== null && next_before >= before))) ||
    records.some((r, i) => i > 0 && r.offset >= records[i - 1].offset) ||
    (next_before !== null &&
      (scanned === 0 ||
        next_before > head_offset ||
        records.some((r) => r.offset < next_before)))
  )
    return invalid();
  return {
    generated_at: time(row.generated_at),
    outcome: expected,
    head_offset,
    scanned,
    next_before,
    records,
  };
}
