export const executionProject = '项目/Studiolo';
export const executionHash = 'a'.repeat(64);
export const executionId = (n = 1) => String(n).padStart(26, '0');
export function executionBoard(status = 'queued') {
  const active = [
    'claimed',
    'running',
    'cancel_requested',
    'committing',
  ].includes(status);
  const attempted =
    active || !['draft', 'queued', 'needs_reconfirm'].includes(status);
  const attempt = `A-${executionId(101)}`;
  return {
    task: {
      task_id: executionId(),
      spec: {
        role: 'builder',
        adapter: 'codex',
        input_paths: [],
        allow_web: [],
        allow_db: [],
        retry_policy: { max_attempts: 1, backoff_seconds: 0, retry_on: [] },
        timeout: 600,
        project: executionProject,
        parent: null as string | null,
        title: 'Runner 契约任务',
        instruction: null,
      },
      status,
      active_attempt: active ? attempt : null,
      last_attempt: attempted && !active ? attempt : null,
      input_snapshot: attempted ? executionHash : null,
    },
    board: {
      task_id: executionId(),
      definition: {
        acceptance_criteria: ['真实候选已核对'],
        files_owned: ['项目/Studiolo/03-记忆'],
        max_children: 8,
        policy_hash: executionHash,
        parent: null as string | null,
        artifact_target: null,
      },
      depth: 0,
      created_by_instance: 'registered-codex-instance',
      handoff: null,
      receipt: null as unknown,
    },
    uses_old_policy: false,
    holder: attempted ? 'registered-codex-instance' : null,
    progress: attempted
      ? { tool_calls: 5, edits: 2, last_progress_ms: Date.now() }
      : null,
    attempt: attempted
      ? {
          attempt_id: attempt,
          task_id: executionId(),
          fence: 9,
          phase: active ? status : 'completed',
          worker_id: 'registered-codex-instance',
          snapshot_id: executionHash,
          lease_until_ms: Date.now() + 120_000,
          runner_ref_json: null,
          terminal: null,
        }
      : null,
    candidate_stages: [] as unknown[],
    candidate_receipt: (status === 'needs_review'
      ? {
          manifest_path: '项目/Studiolo/03-记忆/manifest.json',
          manifest_revision: executionHash,
          artifacts: [
            {
              path: '项目/Studiolo/03-记忆/result.md',
              revision: executionHash,
              sha256: executionHash,
            },
          ],
        }
      : null) as unknown,
  };
}
export function executionRun(state = 'running') {
  return {
    task_id: executionId(),
    attempt_id: `A-${executionId(101)}`,
    adapter: 'codex',
    state,
    progress: { tool_calls: 5, edits: 2, phase: 'pacing' },
  };
}
export function executionReceipt() {
  return {
    status: 'completed',
    artifacts: [
      {
        path: '项目/Studiolo/03-记忆/result.md',
        revision: executionHash,
        sha256: executionHash,
      },
    ],
    conflicts: [],
    unlanded: [],
    acceptance: [
      {
        criterion: '真实候选已核对',
        result: 'pass',
        evidence: [
          {
            path: '项目/Studiolo/03-记忆/result.md',
            revision: executionHash,
            sha256: executionHash,
          },
        ],
      },
    ],
    remaining: [],
    elapsed_ms: 1400,
    tool_calls: 5,
    learned: ['已核对产物'],
    closeout: {
      learned: ['已核对产物'],
      adr_required: false,
      rule_change_required: false,
      destinations: ['项目/Studiolo/03-记忆'],
    },
  };
}
export function executionEvent(offset = '1') {
  return {
    offset,
    at: '2026-09-20T10:00:00Z',
    action: 'runner.call',
    phase: 'outcome',
    attempt_id: `A-${executionId(101)}`,
    details: { model: 'configured-model', decision: 'allow' },
  };
}
export function executionResponse(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
