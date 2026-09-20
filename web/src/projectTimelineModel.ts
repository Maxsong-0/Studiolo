type Event = {
  offset: string;
  at: string;
  task_id: string;
  action: string;
  phase: string;
  attempt_id: string | null;
};
type Log = { path: string; title: string; date: string };
export type Timeline = {
  project: string;
  from: string;
  to: string;
  events: Event[];
  logs: Log[];
  undated_logs: number;
  next: string | null;
};
function record(value: unknown, fields: string): Record<string, unknown> {
  if (
    !value ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    Object.keys(value).sort().join(' ') !== fields.split(' ').sort().join(' ')
  )
    throw new Error('项目回放结构无效。');
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 1024): string {
  if (
    typeof value !== 'string' ||
    value.length > max ||
    !value.length ||
    [...value].some((c) => c.charCodeAt(0) < 32)
  )
    throw new Error('项目回放字段无效。');
  return value;
}
function validDate(value: string): boolean {
  return (
    /^\d{4}-\d{2}-\d{2}$/.test(value) &&
    Number.isFinite(Date.parse(`${value}T00:00:00Z`)) &&
    new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) === value
  );
}
function instant(value: string): number {
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/.test(
      value,
    ) ||
    !validDate(value.slice(0, 10)) ||
    !Number.isFinite(Date.parse(value))
  )
    throw new Error('项目回放时间无效。');
  return Date.parse(value);
}
function cursor(value: string): boolean {
  return (
    /^(?:0|[1-9]\d*)$/.test(value) &&
    value.length <= 20 &&
    BigInt(value) <= 18446744073709551615n
  );
}
export function timelinePage(
  value: unknown,
  project: string,
  from: string,
  to: string,
): Timeline {
  const start = instant(from),
    end = instant(to);
  if (start > end || end - start > 31 * 86400000)
    throw new Error('项目回放时间范围无效。');
  const v = record(value, 'project from to events logs undated_logs next');
  if (
    v.project !== project ||
    v.from !== from ||
    v.to !== to ||
    !Array.isArray(v.events) ||
    v.events.length > 1000 ||
    !Array.isArray(v.logs) ||
    v.logs.length > 366 ||
    !Number.isSafeInteger(v.undated_logs) ||
    Number(v.undated_logs) < 0
  )
    throw new Error('项目回放归属无效。');
  const events = v.events.map((item) => {
    const e = record(item, 'offset at task_id action phase attempt_id');
    const at = text(e.at);
    const offset = text(e.offset);
    if (
      !cursor(offset) ||
      instant(at) < start ||
      instant(at) > end ||
      !['intent', 'outcome'].includes(String(e.phase))
    )
      throw new Error('项目事件范围无效。');
    return {
      offset,
      at,
      task_id: text(e.task_id),
      action: text(e.action),
      phase: text(e.phase),
      attempt_id: e.attempt_id === null ? null : text(e.attempt_id),
    };
  });
  for (let i = 1; i < events.length; i++)
    if (BigInt(events[i].offset) <= BigInt(events[i - 1].offset))
      throw new Error('回放事件顺序无效。');
  const logs = v.logs.map((item) => {
    const l = record(item, 'path title date');
    const path = text(l.path);
    const date = text(l.date);
    if (
      !path.startsWith(`${project}/02-日志/`) ||
      path.split('/').some((p) => !p || p === '.' || p === '..') ||
      !validDate(date) ||
      date < from.slice(0, 10) ||
      date > to.slice(0, 10)
    )
      throw new Error('项目日志范围无效。');
    return { path, title: text(l.title), date };
  });
  const next = v.next === null ? null : text(v.next);
  if (
    next !== null &&
    (!cursor(next) || events.some((e) => BigInt(e.offset) > BigInt(next)))
  )
    throw new Error('回放分页无效。');
  return {
    project,
    from,
    to,
    events,
    logs,
    undated_logs: Number(v.undated_logs),
    next,
  };
}
