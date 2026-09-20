/** Closed BoardView decoder. A partial or inconsistent response never means an empty board. */
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
] as const;
const phases = [
  'claimed',
  'running',
  'cancel_requested',
  'committing',
  'completed',
  'failed',
  'cancelled',
  'abandoned',
] as const;
export type BoardStatus = (typeof statuses)[number];
export type Pointer = { path: string; revision: string; sha256: string };
export type Receipt = {
  status: 'completed' | 'failed' | 'stopped' | 'abandoned';
  artifacts: Pointer[];
  conflicts: string[];
  unlanded: { path: string; sha256: string }[];
  acceptance: {
    criterion: string;
    result: 'pass' | 'fail' | 'unverified';
    evidence: Pointer[];
  }[];
  remaining: string[];
  elapsedMs: number;
  toolCalls: number;
  learned: string[];
  closeout: {
    learned: string[];
    adrRequired: boolean;
    ruleChangeRequired: boolean;
    destinations: string[];
  };
};
export type BoardItem = {
  id: string;
  title: string;
  project: string;
  status: BoardStatus;
  parent: string | null;
  depth: number;
  creator: string | null;
  adapter: 'stub' | 'consumer' | 'claude' | 'codex';
  candidateStages: {
    attemptId: string;
    fence: number;
    pointer: { id: string; sha256: string };
    reason: string;
  }[];
  candidateReceipt: {
    manifestPath: string;
    manifestRevision: string;
    artifacts: Pointer[];
  } | null;
  holder: string | null;
  instanceLineages: { id: string; agentType: string }[][];
  paths: string[];
  criteria: string[];
  oldPolicy: boolean;
  policyHash: string;
  maxChildren: number;
  progress: { tools: number; edits: number; lastProgressMs: number } | null;
  attempt: {
    id: string;
    fence: number;
    phase: (typeof phases)[number];
    worker: string;
    leaseUntilMs: number;
    active: boolean;
  } | null;
  receipt: Receipt | null;
};
export type BoardNode = { item: BoardItem; children: BoardNode[] };
function invalid(): never {
  throw new Error('黑板响应格式或归属关系无效，实时状态不可确认。');
}
function object(value: unknown, fields?: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return invalid();
  const row = value as Record<string, unknown>;
  if (fields) {
    const expected = fields.split(' ');
    if (
      Object.keys(row).length !== expected.length ||
      expected.some((key) => !Object.hasOwn(row, key))
    )
      return invalid();
  }
  return row;
}
function rawText(value: unknown, max = 4096): string {
  if (typeof value !== 'string' || value.length > max) return invalid();
  return value;
}
function text(value: unknown, max = 4096): string {
  const result = rawText(value, max);
  if (!result.trim()) return invalid();
  return result;
}
function integer(value: unknown, max = Number.MAX_SAFE_INTEGER): number {
  if (
    typeof value !== 'number' ||
    !Number.isSafeInteger(value) ||
    value < 0 ||
    value > max
  )
    return invalid();
  return value;
}
function timestamp(value: unknown) {
  return integer(value, 8_640_000_000_000_000);
}
function bool(value: unknown): boolean {
  if (typeof value !== 'boolean') return invalid();
  return value;
}
function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function list<T>(value: unknown, parse: (value: unknown) => T, max = 256): T[] {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map(parse);
}
function strings(value: unknown) {
  return list(value, (v) => text(v));
}
function choice<T extends string>(value: unknown, values: readonly T[]): T {
  if (typeof value !== 'string' || !values.includes(value as T))
    return invalid();
  return value as T;
}
function hash(value: unknown) {
  const v = text(value, 64);
  if (!/^[a-fA-F0-9]{64}$/.test(v)) return invalid();
  return v;
}
function id(value: unknown) {
  const v = text(value, 26);
  if (!/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/.test(v)) return invalid();
  return v;
}
function attemptId(value: unknown) {
  const v = text(value, 28);
  if (!v.startsWith('A-')) return invalid();
  id(v.slice(2));
  return v;
}
function path(value: unknown) {
  const v = text(value);
  if (
    v.normalize('NFC') !== v ||
    v.includes('\\') ||
    Array.from(v).some(
      (character) =>
        character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
    ) ||
    v.split('/').some((part) => !part || part === '.' || part === '..')
  )
    return invalid();
  return v;
}
function pointer(value: unknown): Pointer {
  const v = object(value, 'path revision sha256');
  return {
    path: path(v.path),
    revision: text(v.revision, 256),
    sha256: hash(v.sha256),
  };
}
function receipt(value: unknown, criteria: string[]): Receipt {
  const v = object(
    value,
    'status artifacts conflicts unlanded acceptance remaining elapsed_ms tool_calls learned closeout',
  );
  const close = object(
    v.closeout,
    'learned adr_required rule_change_required destinations',
  );
  const acceptance = list(v.acceptance, (entry) => {
    const a = object(entry, 'criterion result evidence');
    return {
      criterion: text(a.criterion),
      result: choice(a.result, ['pass', 'fail', 'unverified'] as const),
      evidence: list(a.evidence, pointer),
    };
  });
  if (
    acceptance.length !== criteria.length ||
    acceptance.some((a, i) => a.criterion !== criteria[i])
  )
    return invalid();
  const result: Receipt = {
    status: choice(v.status, [
      'completed',
      'failed',
      'stopped',
      'abandoned',
    ] as const),
    artifacts: list(v.artifacts, pointer),
    conflicts: list(v.conflicts, path),
    unlanded: list(v.unlanded, (entry) => {
      const p = object(entry, 'path sha256');
      return { path: path(p.path), sha256: hash(p.sha256) };
    }),
    acceptance,
    remaining: strings(v.remaining),
    elapsedMs: integer(v.elapsed_ms),
    toolCalls: integer(v.tool_calls),
    learned: strings(v.learned),
    closeout: {
      learned: strings(close.learned),
      adrRequired: bool(close.adr_required),
      ruleChangeRequired: bool(close.rule_change_required),
      destinations: list(close.destinations, path),
    },
  };
  if (
    result.conflicts.length ||
    (result.status === 'completed' &&
      (result.acceptance.some((a) => a.result !== 'pass') ||
        result.unlanded.length ||
        result.remaining.length))
  )
    return invalid();
  return result;
}
function handoff(value: unknown) {
  const v = object(
    value,
    'goal constraints decided undecided required_files acceptance_criteria forbidden',
  );
  text(v.goal);
  strings(v.decided);
  strings(v.undecided);
  strings(v.acceptance_criteria);
  strings(v.forbidden);
  list(v.constraints, (entry) => {
    const c = object(entry, 'rule_id version_hash');
    text(c.rule_id);
    hash(c.version_hash);
  });
  list(v.required_files, (entry) => {
    const f = object(entry, 'path revision');
    path(f.path);
    text(f.revision, 256);
  });
}
function item(value: unknown, expectedProject?: string): BoardItem {
  const v = object(
    value,
    'task board uses_old_policy holder progress attempt candidate_stages candidate_receipt' +
      (Object.prototype.hasOwnProperty.call(value, 'instance_lineages')
        ? ' instance_lineages'
        : ''),
  );
  const task = object(
    v.task,
    'task_id spec status active_attempt last_attempt input_snapshot',
  );
  const spec = object(
    task.spec,
    'role adapter input_paths allow_web allow_db retry_policy timeout project parent title instruction',
  );
  const board = object(
    v.board,
    'task_id definition depth created_by_instance handoff receipt',
  );
  const definition = object(
    board.definition,
    'acceptance_criteria files_owned max_children policy_hash parent artifact_target',
  );
  const taskId = id(task.task_id),
    parent = nullable(definition.parent, id),
    project = text(spec.project);
  const active = nullable(task.active_attempt, attemptId),
    last = nullable(task.last_attempt, attemptId);
  const status = choice(task.status, statuses),
    holder = nullable(v.holder, text);
  if (
    id(board.task_id) !== taskId ||
    nullable(spec.parent, id) !== parent ||
    (expectedProject !== undefined && project !== expectedProject)
  )
    return invalid();
  text(spec.role);
  const adapter = choice(spec.adapter, [
    'stub',
    'consumer',
    'claude',
    'codex',
  ] as const);
  list(spec.input_paths, path);
  strings(spec.allow_web);
  strings(spec.allow_db);
  nullable(spec.instruction, rawText);
  if (!integer(spec.timeout)) return invalid();
  const retry = object(
    spec.retry_policy,
    'max_attempts backoff_seconds retry_on',
  );
  if (!integer(retry.max_attempts)) return invalid();
  integer(retry.backoff_seconds);
  strings(retry.retry_on);
  nullable(task.input_snapshot, hash);
  nullable(definition.artifact_target, text);
  const criteria = strings(definition.acceptance_criteria),
    paths = list(definition.files_owned, path);
  if (
    !criteria.length ||
    new Set(criteria).size !== criteria.length ||
    new Set(paths).size !== paths.length
  )
    return invalid();
  const parsedAttempt = nullable(v.attempt, (entry) => {
    const a = object(
      entry,
      'attempt_id task_id fence phase worker_id snapshot_id lease_until_ms runner_ref_json terminal',
    );
    const attempt = {
      id: attemptId(a.attempt_id),
      fence: integer(a.fence),
      phase: choice(a.phase, phases),
      worker: text(a.worker_id),
      leaseUntilMs: timestamp(a.lease_until_ms),
      active: active !== null,
    };
    if (
      !attempt.fence ||
      id(a.task_id) !== taskId ||
      attempt.id !== (active ?? last) ||
      attempt.worker !== holder
    )
      return invalid();
    hash(a.snapshot_id);
    // Opaque server-only evidence is never interpreted as an authorization or rendered.
    nullable(a.runner_ref_json, object);
    nullable(a.terminal, (entry) =>
      object(entry, 'report_json artifacts_json'),
    );
    const inFlight = [
      'claimed',
      'running',
      'cancel_requested',
      'committing',
    ].includes(attempt.phase);
    if (
      attempt.active !== inFlight ||
      (attempt.active && attempt.phase !== status)
    )
      return invalid();
    return attempt;
  });
  if (
    (active !== null || last !== null) !== (parsedAttempt !== null) ||
    (parsedAttempt === null && holder !== null)
  )
    return invalid();
  if (
    ['claimed', 'running', 'cancel_requested', 'committing'].includes(
      status,
    ) !==
    (active !== null)
  )
    return invalid();
  const progress = nullable(v.progress, (entry) => {
    const p = object(entry, 'tool_calls edits last_progress_ms');
    return {
      tools: integer(p.tool_calls),
      edits: integer(p.edits),
      lastProgressMs: timestamp(p.last_progress_ms),
    };
  });
  if (progress && !parsedAttempt) return invalid();
  nullable(board.handoff, handoff);
  const parsedReceipt = nullable(board.receipt, (entry) =>
    receipt(entry, criteria),
  );
  if (parsedReceipt) {
    const terminalStatus = {
      completed: 'succeeded',
      failed: 'failed',
      stopped: 'cancelled',
      abandoned: 'abandoned',
    };
    if (status !== terminalStatus[parsedReceipt.status] || active !== null)
      return invalid();
  }
  return {
    id: taskId,
    title: nullable(spec.title, rawText) || taskId,
    project,
    status,
    parent,
    depth: integer(board.depth, 3),
    creator: nullable(board.created_by_instance, text),
    adapter,
    candidateStages: list(v.candidate_stages, (entry) => {
      const stage = object(entry, 'attempt_id fence pointer reason');
      const p = object(stage.pointer, 'id sha256');
      const fence = integer(stage.fence);
      if (!fence) return invalid();
      return {
        attemptId: attemptId(stage.attempt_id),
        fence,
        pointer: { id: hash(p.id), sha256: hash(p.sha256) },
        reason: text(stage.reason),
      };
    }),
    candidateReceipt: nullable(v.candidate_receipt, (entry) => {
      const candidate = object(
        entry,
        'manifest_path manifest_revision artifacts',
      );
      return {
        manifestPath: path(candidate.manifest_path),
        manifestRevision: text(candidate.manifest_revision, 256),
        artifacts: list(candidate.artifacts, pointer),
      };
    }),
    holder,
    instanceLineages:
      v.instance_lineages === undefined
        ? []
        : list(v.instance_lineages, (chain) => {
            const nodes = list(chain, (entry) => {
              const node = object(entry, 'id agent_type');
              return { id: text(node.id), agentType: text(node.agent_type) };
            });
            if (
              !nodes.length ||
              nodes.length > 4 ||
              new Set(nodes.map((n) => n.id)).size !== nodes.length ||
              nodes.some((n) => n.agentType !== nodes[0].agentType) ||
              ![holder, board.created_by_instance].includes(
                nodes[nodes.length - 1].id,
              )
            )
              return invalid();
            return nodes;
          }),
    paths,
    criteria,
    oldPolicy: bool(v.uses_old_policy),
    policyHash: hash(definition.policy_hash),
    maxChildren: integer(definition.max_children, 8),
    progress,
    attempt: parsedAttempt,
    receipt: parsedReceipt,
  };
}
/** Singular replay snapshots may reference a parent outside this page. */
export function boardItem(value: unknown, expectedProject?: string): BoardItem {
  return item(value, expectedProject);
}
/** Validates the exact wire receipt before it is sent; it does not verify evidence content. */
export function boardReceipt(value: unknown, criteria: string[]): Receipt {
  return receipt(value, criteria);
}
export function boardItems(
  value: unknown,
  expectedProject?: string,
): BoardItem[] {
  const items = list(value, (v) => item(v, expectedProject), 1000);
  const byId = new Map(items.map((v) => [v.id, v]));
  if (byId.size !== items.length) return invalid();
  const instanceParents = new Map<
    string,
    { parent: string | null; type: string }
  >();
  for (const current of items)
    for (const chain of current.instanceLineages) {
      chain.forEach((node, index) => {
        const value = {
          parent: index ? chain[index - 1].id : null,
          type: node.agentType,
        };
        const prior = instanceParents.get(node.id);
        if (
          prior &&
          (prior.parent !== value.parent || prior.type !== value.type)
        )
          return invalid();
        instanceParents.set(node.id, value);
      });
    }
  const childCounts = new Map<string, number>();
  for (const current of items) {
    const parent = current.parent === null ? null : byId.get(current.parent);
    if (current.parent === null) {
      if (current.depth !== 0) return invalid();
    } else {
      if (
        !parent ||
        parent.project !== current.project ||
        parent.depth + 1 !== current.depth
      )
        return invalid();
      const count = (childCounts.get(parent.id) ?? 0) + 1;
      if (count > parent.maxChildren) return invalid();
      childCounts.set(parent.id, count);
    }
  }
  return items;
}
export function boardTree(items: BoardItem[]): BoardNode[] {
  const nodes = new Map(
    items.map((item) => [item.id, { item, children: [] as BoardNode[] }]),
  );
  const roots: BoardNode[] = [];
  for (const node of nodes.values()) {
    if (node.item.parent === null) roots.push(node);
    else nodes.get(node.item.parent)?.children.push(node);
  }
  return roots;
}
/** Local clock only; the server remains the authority for all claims and writes. */
export function boardLease(
  item: BoardItem,
  nowMs: number,
): 'none' | 'expired' | 'current' | 'revoked' | 'settling' | 'ended' {
  if (!item.attempt) return 'none';
  if (!item.attempt.active) return 'ended';
  if (item.attempt.phase === 'cancel_requested') return 'revoked';
  if (item.attempt.phase === 'committing') return 'settling';
  return item.attempt.leaseUntilMs <= nowMs ? 'expired' : 'current';
}
