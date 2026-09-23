/**
 * 黑板事件（权威审计里带任务主体的那一部分）：供项目时间回放、跨项目汇总与单任务审计回放共用。
 *
 * 排期对齐画布「今天的时间线」：M4 接续线 08:30–10:06 与 14:00–14:30、全项目收尾线 09:00–11:24 与
 * 16:12–18:30、接入线 12:48–15:36、重设计线 19:31 认领 T-130 后持续到 19:57；主人的审批在
 * 11:42、13:00、14:30（外加 10:57 简报站验收）。T-108 那一次执行走完九步里的前七步（阶段表见
 * web/src/executionView.ts：启动、认领、执行体启动、工作、节奏检查、准备收尾、提交候选），停在第 8 步
 * 「落地与审阅」等主人，第 9 步「结案」在回执写入后才算走完。T-046 的事件停在四天前，今天没有动静。
 */
import { DAY, HOUR, SECOND, clockAt, iso } from './clock.ts';
import { ATTEMPT, POLICY_HASH, TASK, sha256 } from './ids.ts';
import type { TaskKey } from './ids.ts';

export type Details = Record<string, string | number | boolean | null>;

export interface BoardEvent {
  offset: string;
  at: number;
  task: TaskKey;
  action: string;
  phase: 'intent' | 'outcome';
  attemptId: string | null;
  details: Details;
}

type Draft = Omit<BoardEvent, 'offset'>;

/** 审计偏移从这里起算；每条之间留空，模拟别的项目的记录穿插其间。 */
export const OFFSET_BASE = 48_120;
export const OFFSET_STEP = 3;

const call = (key: TaskKey, index: number) =>
  `gw-${sha256(`call ${key} ${index}`).slice(0, 12)}`;
const model = (key: TaskKey) =>
  key === 'T092' || key === 'T093' || key === 'T046'
    ? { provider: 'openai', model: 'codex' }
    : { provider: 'anthropic', model: 'claude-opus-5' };

function steps(key: TaskKey, time: number, fence: number): Draft[] {
  const attempt = ATTEMPT[key];
  return [
    {
      at: time,
      task: key,
      action: 'task.claim',
      phase: 'intent',
      attemptId: null,
      details: {},
    },
    {
      at: time + SECOND,
      task: key,
      action: 'task.claim',
      phase: 'outcome',
      attemptId: attempt,
      details: {
        fence,
        snapshot_id: sha256(`snapshot ${key}`),
        policy_hash: key === 'T046' ? POLICY_HASH.v1 : POLICY_HASH.v3,
      },
    },
  ];
}

function single(
  key: TaskKey,
  time: number,
  action: string,
  details: Details,
): Draft {
  return {
    at: time,
    task: key,
    action,
    phase: 'outcome',
    attemptId: ATTEMPT[key],
    details,
  };
}

/** 主人的动作（execution.*）：主体只有任务，没有 attempt。 */
function human(
  key: TaskKey,
  time: number,
  action: string,
  outcome: Details,
): Draft[] {
  return [
    {
      at: time,
      task: key,
      action,
      phase: 'intent',
      attemptId: null,
      details: { reason: 'requested' },
    },
    {
      at: time + SECOND,
      task: key,
      action,
      phase: 'outcome',
      attemptId: null,
      details: { reason: 'completed', ...outcome },
    },
  ];
}

function candidates(key: TaskKey, time: number, fence: number): Draft[] {
  return [
    {
      at: time,
      task: key,
      action: 'task.runner_candidates',
      phase: 'intent',
      attemptId: ATTEMPT[key],
      details: { fence },
    },
    {
      at: time + SECOND,
      task: key,
      action: 'task.runner_candidates',
      phase: 'outcome',
      attemptId: ATTEMPT[key],
      details: {
        status: 'needs_review',
        manifest_sha256: sha256(`manifest ${key}`),
      },
    },
  ];
}

const relay = (key: TaskKey, time: number, index: number) =>
  single(key, time, 'gateway.model_relay', {
    call_id: call(key, index),
    ...model(key),
    decision: 'allow',
  });
const tool = (key: TaskKey, time: number, index: number) =>
  single(key, time, 'gateway.tool_call', {
    call_id: call(key, index),
    decision: 'allow',
  });
const beat = (key: TaskKey, time: number, fence: number) =>
  single(key, time, 'task.heartbeat', { fence, state: 'running' });
const started = (key: TaskKey, time: number, fence: number) =>
  single(key, time, 'runner.start', { state: 'started', fence });

/** 全部初始事件，按时间排好并编上审计偏移。 */
export function initialEvents(now: number): BoardEvent[] {
  const h = (hour: number) => clockAt(now, hour);
  const old = now - 4 * DAY - 3 * HOUR;
  const drafts: Draft[] = [
    // Codex 根统筹：四天前认领 T-046 后再无进度。
    ...steps('T046', old, 3),
    started('T046', old + 2 * SECOND, 3),
    relay('T046', old + 40 * SECOND, 1),
    // M4 接续线（Codex）
    ...steps('T093', h(8.5), 1),
    started('T093', h(8.55), 1),
    relay('T093', h(8.7), 1),
    tool('T093', h(9.2), 2),
    beat('T093', h(9.4), 1),
    ...candidates('T093', h(10.1), 1),
    ...human('T093', h(11.7), 'execution.review', {
      status: 'needs_revision',
    }),
    ...steps('T092', h(14.0), 2),
    started('T092', h(14.05), 2),
    ...candidates('T092', h(14.25), 2),
    ...human('T092', h(14.5), 'execution.review', { status: 'approved' }),
    // 全项目收尾线（Claude）
    ...steps('T109', h(9.0), 1),
    started('T109', h(9.05), 1),
    relay('T109', h(9.3), 1),
    beat('T109', h(10.0), 1),
    tool('T109', h(10.6), 2),
    beat('T109', h(11.4), 1),
    beat('T109', h(16.2), 1),
    tool('T109', h(17.3), 3),
    beat('T109', h(18.5), 1),
    ...steps('B021', h(10.3), 1),
    started('B021', h(10.35), 1),
    ...candidates('B021', h(10.85), 1),
    ...human('B021', h(10.95), 'execution.review', { status: 'succeeded' }),
    // 接入线（Claude）
    ...steps('T106', h(12.8), 1),
    started('T106', h(12.85), 1),
    beat('T106', h(13.5), 1),
    relay('T106', h(14.2), 1),
    ...candidates('T106', h(15.1), 1),
    ...human('T108', h(13.0), 'execution.start', {}),
    ...steps('T108', h(13.05), 1),
    started('T108', h(13.08), 1),
    relay('T108', h(13.4), 1),
    tool('T108', h(14.6), 2),
    beat('T108', h(15.0), 1),
    ...candidates('T108', h(15.6), 1),
    // 重设计线（Claude）：19:31 认领 T-130
    ...steps('T130', h(19.52), 1),
    started('T130', h(19.55), 1),
    relay('T130', h(19.7), 1),
    beat('T130', h(19.85), 1),
    tool('T130', h(19.95), 2),
  ];
  return drafts
    .map((draft, index) => ({ draft, index }))
    .sort(
      (left, right) =>
        left.draft.at - right.draft.at || left.index - right.index,
    )
    .map(({ draft }, index) => ({
      ...draft,
      offset: String(OFFSET_BASE + index * OFFSET_STEP),
    }));
}

/** 写操作追加的事件：偏移接在最后一条之后。 */
export function appendEvents(
  events: BoardEvent[],
  drafts: Draft[],
): BoardEvent[] {
  let next = Number(events.at(-1)?.offset ?? OFFSET_BASE) + OFFSET_STEP;
  return [
    ...events,
    ...drafts.map((draft) => {
      const event = { ...draft, offset: String(next) };
      next += OFFSET_STEP;
      return event;
    }),
  ];
}

export const humanEvents = human;

export function timelineEvent(event: BoardEvent) {
  return {
    offset: event.offset,
    at: iso(event.at),
    task_id: TASK[event.task],
    action: event.action,
    phase: event.phase,
    attempt_id: event.attemptId,
  };
}

export function replayEvent(event: BoardEvent) {
  return {
    offset: event.offset,
    at: iso(event.at),
    action: event.action,
    phase: event.phase,
    attempt_id: event.attemptId,
    details: event.details,
  };
}
