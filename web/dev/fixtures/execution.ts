/**
 * `/api/v1/execution`：执行服务已配置、未冻结，五个执行体。
 *
 * T-108 是画布上的「执行（示例）7 / 9」：候选已封存（candidate.patch ＋ patch.json 两个产物），
 * 等主人落地与审阅。T-093 预算耗尽后候选留存，未验收。形状以 `executionModel.ts` 为准。
 */
import { ATTEMPT, TASK } from './ids.ts';
import type { TaskKey } from './ids.ts';

export type RunState =
  'running' | 'candidate_review' | 'candidate_retained' | 'stopped' | 'failed';
export type RunPhase = 'working' | 'pacing' | 'wrapup' | 'exhausted';

export interface RunRecord {
  key: TaskKey;
  adapter: 'claude' | 'codex';
  state: RunState;
  tools: number;
  edits: number;
  phase: RunPhase;
}

export function initialRuns(): RunRecord[] {
  return [
    {
      key: 'T130',
      adapter: 'claude',
      state: 'running',
      tools: 212,
      edits: 38,
      phase: 'working',
    },
    {
      key: 'T109',
      adapter: 'claude',
      state: 'running',
      tools: 96,
      edits: 14,
      phase: 'pacing',
    },
    {
      key: 'T108',
      adapter: 'claude',
      state: 'candidate_review',
      tools: 143,
      edits: 27,
      phase: 'wrapup',
    },
    {
      key: 'T106',
      adapter: 'claude',
      state: 'candidate_review',
      tools: 88,
      edits: 19,
      phase: 'wrapup',
    },
    {
      key: 'T093',
      adapter: 'codex',
      state: 'candidate_retained',
      tools: 61,
      edits: 9,
      phase: 'exhausted',
    },
  ];
}

export function runWire(run: RunRecord) {
  return {
    task_id: TASK[run.key],
    attempt_id: ATTEMPT[run.key],
    adapter: run.adapter,
    state: run.state,
    progress: { tool_calls: run.tools, edits: run.edits, phase: run.phase },
  };
}

/** 后端缺二次认证时的原话（`auth_http/execution.rs`），界面据此提示先完成 One-time Code。 */
export const L3_REQUIRED = '请先完成二次认证（One-time Code）';
