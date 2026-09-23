/**
 * `GET /api/v1/board/overview` 的封闭解码器（T-130 第 10 节；契约 docs/contracts/board-overview.md）。
 * 每层字段集合必须与契约逐字相同，取值、长度、时间与相互印证任一不符，整份汇总判无效：
 * 部分或不一致的响应绝不当成空黑板显示。
 */
import type { BoardStatus } from './boardModel';

export const BOARD_OVERVIEW_INVALID = '黑板汇总格式无效，实时状态不可确认。';
export const BOARD_OVERVIEW_DEFAULT_WINDOW_HOURS = 24;
export const BOARD_OVERVIEW_MAX_WINDOW_HOURS = 168;
export const BOARD_OVERVIEW_MAX_ITEMS = 500;
export const BOARD_OVERVIEW_MAX_EVENTS = 1000;

const statuses = [
  'draft',
  'queued',
  'claimed',
  'running',
  'committing',
  'cancel_requested',
  'needs_review',
  'needs_revision',
  'needs_reconfirm',
  'approved',
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
] as const satisfies readonly BoardStatus[];
const attemptPhases = [
  'claimed',
  'running',
  'cancel_requested',
  'committing',
  'completed',
  'failed',
  'cancelled',
  'abandoned',
] as const;
const adapters = ['stub', 'consumer', 'claude', 'codex'] as const;
const receiptStatuses = [
  'completed',
  'failed',
  'stopped',
  'abandoned',
] as const;
const eventPhases = ['intent', 'outcome'] as const;

export type BoardOverviewStatus = (typeof statuses)[number];
export type BoardOverviewAttemptPhase = (typeof attemptPhases)[number];
export type BoardOverviewAdapter = (typeof adapters)[number];
export type BoardOverviewReceiptStatus = (typeof receiptStatuses)[number];
export type BoardOverviewEventPhase = (typeof eventPhases)[number];

/** 尝试仍在进行的四个阶段：任务状态取这四个值时必有活跃尝试，反之亦然。 */
const inFlight: readonly string[] = [
  'claimed',
  'running',
  'cancel_requested',
  'committing',
];
/** 已结束的四个任务状态；`approved` 与各 `needs_*` 仍算未结束。 */
const terminal: readonly string[] = [
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
];
/** 回执结论对应的任务终态，与黑板视图同一张表。 */
const receiptOutcome: Record<BoardOverviewReceiptStatus, BoardOverviewStatus> =
  {
    completed: 'succeeded',
    failed: 'failed',
    stopped: 'cancelled',
    abandoned: 'abandoned',
  };

const TOP_FIELDS = [
  'generated_at',
  'window_from',
  'window_to',
  'policy_hash',
  'items',
  'items_truncated',
  'events',
  'events_truncated',
];
const ITEM_FIELDS = [
  'id',
  'title',
  'project',
  'status',
  'parent',
  'adapter',
  'holder',
  'holder_lineage',
  'paths',
  'attempt',
  'progress',
  'receipt_status',
];
const LINEAGE_FIELDS = ['id', 'agent_type'];
const ATTEMPT_FIELDS = ['phase', 'worker', 'active', 'lease_until_ms'];
const PROGRESS_FIELDS = ['tools', 'edits', 'last_progress_ms'];
const EVENT_FIELDS = [
  'at',
  'project',
  'task_id',
  'action',
  'phase',
  'attempt_id',
];

/** 标题：创建时整份工作项定义不超过 64 KiB，标题不可能更长。 */
const TITLE_MAX = 65_536;
const PATH_MAX = 4096;
/** 实例 id：认领时限定不超过 256 字节。 */
const INSTANCE_MAX = 256;
const ACTION_MAX = 1024;
/** 实例链最深四层，工作项最多 256 条独占路径。 */
const LINEAGE_MAX = 4;
const PATHS_MAX = 256;
const HOUR_MS = 3_600_000;

export type BoardOverviewLineageNode = { id: string; agentType: string };
export type BoardOverviewAttempt = {
  phase: BoardOverviewAttemptPhase;
  worker: string;
  active: boolean;
  leaseUntilMs: number;
};
export type BoardOverviewProgress = {
  tools: number;
  edits: number;
  lastProgressMs: number;
};
export type BoardOverviewItem = {
  id: string;
  title: string;
  project: string;
  status: BoardOverviewStatus;
  parent: string | null;
  adapter: BoardOverviewAdapter;
  holder: string | null;
  /** 当前或最近一次尝试持有者的实例链，根→叶；无持有者时为空。 */
  holderLineage: BoardOverviewLineageNode[];
  paths: string[];
  attempt: BoardOverviewAttempt | null;
  progress: BoardOverviewProgress | null;
  receiptStatus: BoardOverviewReceiptStatus | null;
};
export type BoardOverviewEvent = {
  at: string;
  project: string;
  taskId: string;
  action: string;
  phase: BoardOverviewEventPhase;
  attemptId: string | null;
};
export type BoardOverview = {
  generatedAt: string;
  windowFrom: string;
  windowTo: string;
  policyHash: string;
  /** 全部未结束工作项，加窗口内有事件的已结束项；按（项目, id）升序。 */
  items: BoardOverviewItem[];
  itemsTruncated: boolean;
  /** 窗口内全部项目的任务事件，按时间升序；超限时保留最新的。 */
  events: BoardOverviewEvent[];
  eventsTruncated: boolean;
};

function invalid(): never {
  throw new Error(BOARD_OVERVIEW_INVALID);
}
/** 字段集合必须与 `fields` 逐字相同：多一个、少一个都判无效。 */
function record(
  value: unknown,
  fields: readonly string[],
): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>;
  if (
    Object.keys(row).length !== fields.length ||
    fields.some((key) => !Object.hasOwn(row, key))
  )
    return invalid();
  return row;
}
function hasControl(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true;
  }
  return false;
}
/** 标识类字段：非空白、有长度上限、不含 C0/C1 控制字符。 */
function token(value: unknown, max: number): string {
  if (
    typeof value !== 'string' ||
    !value.trim() ||
    value.length > max ||
    hasControl(value)
  )
    return invalid();
  return value;
}
/**
 * 人写标题是自由文本：服务端已把缺失或全空白回落为任务 id，这里只核长度与非空白。
 * 创建接口不限制标题字符，黑板视图同样照收；按文本节点渲染，控制字符不构成注入。
 */
function title(value: unknown): string {
  if (typeof value !== 'string' || !value.trim() || value.length > TITLE_MAX)
    return invalid();
  return value;
}
function taskId(value: unknown): string {
  const v = token(value, 26);
  if (!/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/.test(v)) return invalid();
  return v;
}
function attemptId(value: unknown): string {
  const v = token(value, 28);
  if (!/^A-[0-7][0-9A-HJKMNP-TV-Z]{25}$/.test(v)) return invalid();
  return v;
}
/** 与 VaultPath 同一语法：NFC、无反斜杠、无空段与 `.`/`..`。 */
function path(value: unknown): string {
  const v = token(value, PATH_MAX);
  if (
    v.normalize('NFC') !== v ||
    v.includes('\\') ||
    v.split('/').some((part) => !part || part === '.' || part === '..')
  )
    return invalid();
  return v;
}
function hash(value: unknown): string {
  const v = token(value, 64);
  if (!/^[0-9a-fA-F]{64}$/.test(v)) return invalid();
  return v;
}
function integer(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0)
    return invalid();
  return value;
}
function bool(value: unknown): boolean {
  if (typeof value !== 'boolean') return invalid();
  return value;
}
function choice<T extends string>(value: unknown, values: readonly T[]): T {
  if (typeof value !== 'string' || !values.includes(value as T))
    return invalid();
  return value as T;
}
function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function list<T>(
  value: unknown,
  parse: (value: unknown) => T,
  max: number,
): T[] {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map((entry) => parse(entry));
}
const RFC3339_UTC = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/;
/** RFC3339 UTC（`Z` 结尾，秒以下 0–9 位）；日期与时刻必须真实存在，不接受 24:00 或 2 月 30 日式进位。 */
function instant(value: unknown): number {
  const v = token(value, 40);
  const ms = Date.parse(v);
  if (
    !RFC3339_UTC.test(v) ||
    !Number.isFinite(ms) ||
    new Date(ms).toISOString().slice(0, 19) !== v.slice(0, 19)
  )
    return invalid();
  return ms;
}
/** 按 Unicode 码点比较，与服务端按 UTF-8 字节排序一致（不用 UTF-16 码元顺序）。 */
function compareCodePoints(left: string, right: string): number {
  const a = Array.from(left, (character) => character.codePointAt(0) ?? 0);
  const b = Array.from(right, (character) => character.codePointAt(0) ?? 0);
  for (let index = 0; index < Math.min(a.length, b.length); index += 1)
    if (a[index] !== b[index]) return a[index] - b[index];
  return a.length - b.length;
}

function lineageNode(value: unknown): BoardOverviewLineageNode {
  const v = record(value, LINEAGE_FIELDS);
  return {
    id: token(v.id, INSTANCE_MAX),
    agentType: token(v.agent_type, INSTANCE_MAX),
  };
}
function attempt(value: unknown): BoardOverviewAttempt {
  const v = record(value, ATTEMPT_FIELDS);
  return {
    phase: choice(v.phase, attemptPhases),
    worker: token(v.worker, INSTANCE_MAX),
    active: bool(v.active),
    leaseUntilMs: integer(v.lease_until_ms),
  };
}
function progress(value: unknown): BoardOverviewProgress {
  const v = record(value, PROGRESS_FIELDS);
  return {
    tools: integer(v.tools),
    edits: integer(v.edits),
    lastProgressMs: integer(v.last_progress_ms),
  };
}
/** 根→叶，最多四层、id 不重复、同一 agent 类型，叶子必须是持有者本人。 */
function validChain(
  chain: BoardOverviewLineageNode[],
  holder: string,
): boolean {
  return (
    chain.length > 0 &&
    new Set(chain.map((node) => node.id)).size === chain.length &&
    chain.every((node) => node.agentType === chain[0].agentType) &&
    chain[chain.length - 1].id === holder
  );
}
function item(value: unknown): BoardOverviewItem {
  const v = record(value, ITEM_FIELDS);
  const id = taskId(v.id);
  const status = choice(v.status, statuses);
  const parent = nullable(v.parent, taskId);
  const holder = nullable(v.holder, (entry) => token(entry, INSTANCE_MAX));
  const holderLineage = list(v.holder_lineage, lineageNode, LINEAGE_MAX);
  const paths = list(v.paths, path, PATHS_MAX);
  const current = nullable(v.attempt, attempt);
  const counters = nullable(v.progress, progress);
  const receiptStatus = nullable(v.receipt_status, (entry) =>
    choice(entry, receiptStatuses),
  );
  const active = current?.active ?? false;
  if (
    parent === id ||
    new Set(paths).size !== paths.length ||
    // 持有者、尝试与实例链出自同一次（当前或最近）尝试：要么都有，要么都没有。
    (holder === null) !== (current === null) ||
    (current !== null && current.worker !== holder) ||
    (holder === null
      ? holderLineage.length !== 0
      : !validChain(holderLineage, holder)) ||
    (counters !== null && current === null) ||
    // 活跃尝试与任务状态互相印证，与黑板视图的解码规则一致。
    (current !== null && current.active !== inFlight.includes(current.phase)) ||
    active !== inFlight.includes(status) ||
    (current !== null && current.active && current.phase !== status) ||
    (receiptStatus !== null &&
      (status !== receiptOutcome[receiptStatus] || active))
  )
    return invalid();
  return {
    id,
    title: title(v.title),
    project: path(v.project),
    status,
    parent,
    adapter: choice(v.adapter, adapters),
    holder,
    holderLineage,
    paths,
    attempt: current,
    progress: counters,
    receiptStatus,
  };
}
function event(value: unknown): BoardOverviewEvent & { ms: number } {
  const v = record(value, EVENT_FIELDS);
  return {
    at: token(v.at, 40),
    ms: instant(v.at),
    project: path(v.project),
    taskId: taskId(v.task_id),
    action: token(v.action, ACTION_MAX),
    phase: choice(v.phase, eventPhases),
    attemptId: nullable(v.attempt_id, attemptId),
  };
}

/**
 * 解码跨项目汇总。`windowHours` 给出时，窗口跨度必须恰好等于请求的小时数。
 * 任一字段不符都抛出 `BOARD_OVERVIEW_INVALID`，不返回部分结果。
 */
export function parseBoardOverview(
  value: unknown,
  windowHours?: number,
): BoardOverview {
  const v = record(value, TOP_FIELDS);
  const from = instant(v.window_from);
  const to = instant(v.window_to);
  const generatedAt = token(v.generated_at, 40);
  const span = to - from;
  if (
    generatedAt !== v.window_to ||
    span % HOUR_MS !== 0 ||
    span < HOUR_MS ||
    span > BOARD_OVERVIEW_MAX_WINDOW_HOURS * HOUR_MS ||
    (windowHours !== undefined && span !== windowHours * HOUR_MS)
  )
    return invalid();
  const items = list(v.items, item, BOARD_OVERVIEW_MAX_ITEMS);
  const itemsTruncated = bool(v.items_truncated);
  const events = list(v.events, event, BOARD_OVERVIEW_MAX_EVENTS);
  const eventsTruncated = bool(v.events_truncated);
  const byId = new Map(items.map((entry) => [entry.id, entry]));
  if (byId.size !== items.length) return invalid();
  for (let index = 1; index < items.length; index += 1) {
    const previous = items[index - 1];
    const current = items[index];
    const order =
      compareCodePoints(previous.project, current.project) ||
      compareCodePoints(previous.id, current.id);
    if (order >= 0) return invalid();
  }
  let previousMs = from;
  for (const entry of events) {
    if (entry.ms < previousMs || entry.ms > to) return invalid();
    previousMs = entry.ms;
    const owner = byId.get(entry.taskId);
    // 事件的工作项必在 items 里（未结束项全收，已结束项因窗口内有事件而收）；只有 items 被截断时例外。
    if (owner ? owner.project !== entry.project : !itemsTruncated)
      return invalid();
  }
  // 事件没截断时窗口内全部事件都在：每个已结束项都必须至少有一条。
  const withEvents = new Set(events.map((entry) => entry.taskId));
  if (
    !eventsTruncated &&
    items.some(
      (entry) => terminal.includes(entry.status) && !withEvents.has(entry.id),
    )
  )
    return invalid();
  return {
    generatedAt,
    windowFrom: token(v.window_from, 40),
    windowTo: token(v.window_to, 40),
    policyHash: hash(v.policy_hash),
    items,
    itemsTruncated,
    events: events.map((entry) => ({
      at: entry.at,
      project: entry.project,
      taskId: entry.taskId,
      action: entry.action,
      phase: entry.phase,
      attemptId: entry.attemptId,
    })),
    eventsTruncated,
  };
}
