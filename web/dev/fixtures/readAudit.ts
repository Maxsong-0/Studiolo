/** 固定的审计样本；分页期间时间与offset保持不变，不读取真实账本。 */
import { INSTANCE } from './ids.ts';
import type {
  ReadAuditPage,
  ReadAuditRow,
} from '../../src/agents/readModel.ts';
const KEYS = [
  'outcome',
  'agent_type',
  'instance_id',
  'since',
  'until',
  'path_prefix',
  'before',
];
export function readAuditBody(
  query: Record<string, unknown>,
  start: number,
  now = start,
): ReadAuditPage | null {
  if (
    Object.keys(query).some((k) => !KEYS.includes(k)) ||
    !['read', 'denied'].includes(String(query.outcome))
  )
    return null;
  for (const key of [
    'agent_type',
    'instance_id',
    'since',
    'until',
    'path_prefix',
  ])
    if (query[key] != null && typeof query[key] !== 'string') return null;
  if (query.outcome === 'denied' && query.path_prefix != null) return null;
  if (
    query.before != null &&
    (!Number.isSafeInteger(query.before) || Number(query.before) < 1)
  )
    return null;
  const since =
      query.since == null ? -Infinity : Date.parse(String(query.since)),
    until = query.until == null ? Infinity : Date.parse(String(query.until));
  if (Number.isNaN(since) || Number.isNaN(until) || since > until) return null;
  const upper =
      query.before == null ? 520 : Math.min(Number(query.before) - 1, 520),
    lower = Math.max(upper - 255, 1);
  const records: ReadAuditRow[] = [];
  for (let offset = upper; offset >= lower; offset--) {
    if (offset % 3 !== 0) continue;
    const denied = offset % 9 === 0;
    if (denied !== (query.outcome === 'denied')) continue;
    const timestamp = start - (520 - offset) * 60_000;
    const type = offset % 2 === 0 ? 'claude' : 'codex';
    const row: ReadAuditRow = {
      offset,
      recorded_at: new Date(timestamp).toISOString(),
      agent_type: denied && offset % 18 === 0 ? null : type,
      instance_id:
        denied && offset % 18 === 0
          ? null
          : type === 'claude'
            ? INSTANCE.C1
            : INSTANCE.R2,
      action: 'read_note',
      path: denied ? null : `项目/Studiolo/资料/示例-${offset}.md`,
      revision: denied ? null : 'a'.repeat(64),
      reason: denied ? 'capability_denied' : null,
    };
    if (
      timestamp < since ||
      timestamp > until ||
      (query.agent_type != null && row.agent_type !== query.agent_type) ||
      (query.instance_id != null && row.instance_id !== query.instance_id) ||
      (query.path_prefix != null &&
        !row.path?.startsWith(String(query.path_prefix)))
    )
      continue;
    records.push(row);
  }
  return {
    generated_at: new Date(now).toISOString(),
    outcome: query.outcome as 'read' | 'denied',
    head_offset: 520,
    scanned: upper === 0 ? 0 : upper - lower + 1,
    next_before: upper > 0 && lower > 1 ? lower : null,
    records,
  };
}
