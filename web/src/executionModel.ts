import { boardItem, boardReceipt } from './boardModel';
import type { BoardItem } from './boardModel';

export type RunSummary = {
  task_id: string;
  attempt_id: string;
  adapter: 'claude' | 'codex';
  state:
    | 'running'
    | 'candidate_review'
    | 'candidate_retained'
    | 'stopped'
    | 'failed';
  progress: {
    tool_calls: number;
    edits: number;
    phase: 'working' | 'pacing' | 'wrapup' | 'exhausted';
  };
};
export type ExecutionStatus = {
  configured: boolean;
  frozen: boolean;
  runs: RunSummary[];
};
export type ReplayEvent = {
  offset: string;
  at: string;
  action: string;
  phase: 'intent' | 'outcome';
  attempt_id: string | null;
  details: Record<string, string | number | boolean | null>;
};
export type ReplayPage = {
  task: BoardItem;
  events: ReplayEvent[];
  next: string | null;
};
export type LandingCommand = {
  operation: 'land';
  task_id: string;
  expected_head: string;
  patch_sha256: string;
};
export type LandingResult = {
  task_id: string;
  attempt_id: string;
  patch_sha256: string;
  expected_head: string;
  after_tree: string;
  applied: boolean;
  replayed: boolean;
  conflicts: string[];
};
export type ExecutionCommand =
  | LandingCommand
  | { operation: 'freeze'; frozen: boolean }
  | {
      operation: 'start';
      task_id: string;
      agent_type: 'claude' | 'codex';
      instance_id: string;
    }
  | { operation: 'cancel'; task_id: string }
  | { operation: 'review'; task_id: string; receipt: unknown };
function invalid(): never {
  throw new Error('执行响应格式无效，当前状态不可确认。');
}
function record(value: unknown, keys?: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>;
  if (keys) {
    const names = keys.split(' ');
    if (
      Object.keys(row).length !== names.length ||
      names.some((k) => !Object.hasOwn(row, k))
    )
      return invalid();
  }
  return row;
}
function text(value: unknown, max = 4096): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max)
    return invalid();
  return value;
}
function choice<T extends string>(value: unknown, values: readonly T[]): T {
  if (typeof value !== 'string' || !values.includes(value as T))
    return invalid();
  return value as T;
}
function integer(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0)
    return invalid();
  return value;
}
function boolean(value: unknown): boolean {
  if (typeof value !== 'boolean') return invalid();
  return value;
}
function taskId(value: unknown): string {
  const result = text(value, 26);
  if (!/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/.test(result)) return invalid();
  return result;
}
function attemptId(value: unknown): string {
  const result = text(value, 28);
  if (!result.startsWith('A-')) return invalid();
  taskId(result.slice(2));
  return result;
}
function list<T>(
  value: unknown,
  parse: (value: unknown) => T,
  max: number,
): T[] {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map(parse);
}
function gitOid(value: unknown): string {
  const oid = text(value, 64);
  if (!/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(oid)) return invalid();
  return oid;
}
function sha256(value: unknown): string {
  const hash = text(value, 64);
  if (!/^[a-f0-9]{64}$/.test(hash)) return invalid();
  return hash;
}
function relativeCodePath(value: unknown): string {
  const path = text(value);
  if (
    path.includes('\\') ||
    Array.from(path).some(
      (character) =>
        character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
    ) ||
    /^[A-Za-z]:/.test(path) ||
    path.split('/').some((part) => !part || part === '.' || part === '..')
  )
    return invalid();
  return path;
}
export function landingResult(
  value: unknown,
  command: LandingCommand,
): LandingResult {
  const v = record(
    value,
    'task_id attempt_id patch_sha256 expected_head after_tree applied replayed conflicts',
  );
  const result = {
    task_id: taskId(v.task_id),
    attempt_id: attemptId(v.attempt_id),
    patch_sha256: sha256(v.patch_sha256),
    expected_head: gitOid(v.expected_head),
    after_tree: gitOid(v.after_tree),
    applied: boolean(v.applied),
    replayed: boolean(v.replayed),
    conflicts: list(v.conflicts, relativeCodePath, 4096),
  };
  if (
    result.task_id !== command.task_id ||
    result.expected_head !== command.expected_head ||
    result.patch_sha256 !== command.patch_sha256 ||
    (result.applied && result.conflicts.length > 0) ||
    (result.replayed && !result.applied) ||
    new Set(result.conflicts).size !== result.conflicts.length
  )
    return invalid();
  return result;
}
/** Only one sealed candidate.patch can identify a code landing. */
export function candidatePatch(task: BoardItem) {
  const patches =
    task.candidateReceipt?.artifacts.filter(
      (artifact) => artifact.path.split('/').at(-1) === 'candidate.patch',
    ) ?? [];
  return patches.length === 1 ? patches[0] : null;
}
export function landingCommand(
  task: BoardItem,
  head: string,
  hash: string,
): LandingCommand {
  const patch = candidatePatch(task);
  if (task.status !== 'needs_review' || !task.attempt || task.receipt || !patch)
    throw new Error('只有已封存、待审阅的代码候选可以申请落地。');
  if (!/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(head))
    throw new Error('请填写目标仓库当前完整 HEAD（40 或 64 位小写十六进制）。');
  if (!/^[a-f0-9]{64}$/.test(hash) || hash !== patch.sha256)
    throw new Error('候选 SHA-256 必须与已封存的 candidate.patch 指针一致。');
  return {
    operation: 'land',
    task_id: task.id,
    expected_head: head,
    patch_sha256: hash,
  };
}
export function runSummary(value: unknown): RunSummary {
  const row = record(value, 'task_id attempt_id adapter state progress');
  const progress = record(row.progress, 'tool_calls edits phase');
  return {
    task_id: taskId(row.task_id),
    attempt_id: attemptId(row.attempt_id),
    adapter: choice(row.adapter, ['claude', 'codex'] as const),
    state: choice(row.state, [
      'running',
      'candidate_review',
      'candidate_retained',
      'stopped',
      'failed',
    ] as const),
    progress: {
      tool_calls: integer(progress.tool_calls),
      edits: integer(progress.edits),
      phase: choice(progress.phase, [
        'working',
        'pacing',
        'wrapup',
        'exhausted',
      ] as const),
    },
  };
}
export function executionStatus(value: unknown): ExecutionStatus {
  const row = record(value, 'configured frozen runs');
  const result = {
    configured: boolean(row.configured),
    frozen: boolean(row.frozen),
    runs: list(row.runs, runSummary, 1000),
  };
  if (
    (!result.configured && (!result.frozen || result.runs.length)) ||
    new Set(result.runs.map((run) => run.task_id)).size !== result.runs.length
  )
    return invalid();
  return result;
}
function offset(value: unknown): string {
  const v = text(value, 20);
  if (!/^(0|[1-9]\d*)$/.test(v) || BigInt(v) > 18_446_744_073_709_551_615n)
    return invalid();
  return v;
}
const detailKeys = new Set([
  'state',
  'status',
  'fence',
  'call_id',
  'provider',
  'model',
  'snapshot_id',
  'policy_hash',
  'decision',
  'reason',
  'receipt_sha256',
  'manifest_sha256',
]);
function event(value: unknown): ReplayEvent {
  const v = record(value, 'offset at action phase attempt_id details');
  const details = record(v.details);
  for (const [key, value] of Object.entries(details)) {
    if (
      !detailKeys.has(key) ||
      !(
        value === null ||
        typeof value === 'boolean' ||
        (typeof value === 'string' && value.length <= 4096) ||
        (typeof value === 'number' && Number.isSafeInteger(value))
      )
    )
      return invalid();
  }
  const at = text(v.at, 64);
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/.test(
      at,
    ) ||
    !Number.isFinite(Date.parse(at))
  )
    return invalid();
  return {
    offset: offset(v.offset),
    at,
    action: text(v.action, 256),
    phase: choice(v.phase, ['intent', 'outcome'] as const),
    attempt_id: v.attempt_id === null ? null : attemptId(v.attempt_id),
    details: details as ReplayEvent['details'],
  };
}
export function replayPage(
  value: unknown,
  id: string,
  project: string,
  after: string | null,
): ReplayPage {
  const v = record(value, 'task events next');
  const task = boardItem(v.task, project);
  const events = list(v.events, event, 100);
  let previous = BigInt(after === null ? '0' : offset(after));
  for (const row of events) {
    if (BigInt(row.offset) <= previous) return invalid();
    previous = BigInt(row.offset);
  }
  const next = v.next === null ? null : offset(v.next);
  if (
    task.id !== id ||
    (next !== null && (events.length !== 100 || next !== events.at(-1)?.offset))
  )
    return invalid();
  return { task, events, next };
}
export function startCommand(task: BoardItem): ExecutionCommand {
  if (
    !task.creator ||
    !['draft', 'queued'].includes(task.status) ||
    (task.adapter !== 'claude' && task.adapter !== 'codex')
  )
    throw new Error('任务必须已登记执行适配器与创建实例，并处于待认领状态。');
  return {
    operation: 'start',
    task_id: task.id,
    agent_type: task.adapter,
    instance_id: task.creator,
  };
}
/** Deliberately incomplete: decisions and observations must be supplied by the reviewer. */
export function reviewTemplate(task: BoardItem): string {
  return JSON.stringify(
    {
      status: '请选择 completed / failed / stopped / abandoned',
      artifacts: task.candidateReceipt?.artifacts ?? [],
      conflicts: [],
      unlanded: [],
      acceptance: task.criteria.map((criterion) => ({
        criterion,
        result: 'unverified',
        evidence: [],
      })),
      remaining: [],
      elapsed_ms: null,
      tool_calls: null,
      learned: [],
      closeout: {
        learned: [],
        adr_required: null,
        rule_change_required: null,
        destinations: [],
      },
    },
    null,
    2,
  );
}
export function reviewReceipt(source: string, task: BoardItem): unknown {
  if (new TextEncoder().encode(source).length > 120_000)
    throw new Error('回执过大，请保留产物指针与必要说明。');
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    throw new Error('回执不是有效 JSON。');
  }
  try {
    boardReceipt(value, task.criteria);
  } catch {
    throw new Error(
      '回执字段不完整或不符合任务验收项。请逐条填写 result/evidence、耗时、调用数及 closeout；completed 必须全部 pass 且无未落地或剩余事项。',
    );
  }
  return value;
}
