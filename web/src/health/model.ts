/** 同步健康的封闭模型；64位计数保留十进制字符串，避免浏览器整数精度丢失。 */
export interface BridgeHealth {
  lastBatch: { sequence: string; closedAt: string | null } | null;
  fused: boolean | null;
  unknownJobs: string | null;
  quiescenceValid: boolean | null;
  conflictCopies: string | null;
  mirrorBytes: string | null;
  lastExit:
    | 'clean'
    | 'non_zero'
    | 'signal'
    | 'deadline_exceeded'
    | 'owner_stopped'
    | 'unconfirmed'
    | null;
  recovery: 'idle' | 'forced_mirror_remote' | 'backoff' | null;
  residueGuard: 'clear' | 'blocked' | null;
  mode: 'mirror-remote' | 'bidirectional' | null;
  configMatches: boolean | null;
  /** 主库没开文件操作、整批暂停时被挡的删除数；null 即还没有一轮证明过（BUG-046）。 */
  deletesBlocked: string | null;
  /** 一轮要比对的路径数与上限；null 即还没有一轮算到过（BUG-047）。 */
  pathUsage: BudgetUsage | null;
  /** 一轮要比对的内容字节数（基线、主库、镜像三方各算一遍）与上限；null 即还没有一轮算到过。 */
  byteUsage: BudgetUsage | null;
}
export interface BudgetUsage {
  used: string;
  budget: string;
}
export interface ManualQuota {
  accountLimitBytes: string;
  usedBytesManual: string;
  recordedAt: string | null;
  asOf: string | null;
  state: 'normal' | 'degraded' | 'unknown';
  reason: 'within' | 'warning' | 'exceeded' | 'clock' | 'stale' | 'invalid';
}
export function object(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
export function keys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const actual = Object.keys(value);
  return (
    actual.length === expected.length &&
    expected.every((key) => Object.hasOwn(value, key))
  );
}
export function decimal(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^(0|[1-9]\d{0,19})$/.test(value) &&
    BigInt(value) <= 18446744073709551615n
  );
}
function timestamp(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z$/.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}
const optional = (value: unknown, valid: (value: unknown) => boolean) =>
  value === null || valid(value);
const oneOf = <T extends string>(
  value: unknown,
  choices: readonly T[],
): value is T => typeof value === 'string' && choices.includes(value as T);

/** 一项用量：`null`，或恰好两个 u64 十进制字符串；形状不对返回 `undefined`。 */
function usage(value: unknown): BudgetUsage | null | undefined {
  if (value === null) return null;
  if (
    !object(value) ||
    !keys(value, ['used', 'budget']) ||
    !decimal(value.used) ||
    !decimal(value.budget)
  )
    return undefined;
  return { used: value.used, budget: value.budget };
}
export function parseBridge(value: unknown): BridgeHealth | null {
  if (
    !object(value) ||
    !keys(value, [
      'last_batch',
      'fused',
      'unknown_jobs',
      'quiescence_valid',
      'conflict_copies',
      'mirror_bytes',
      'last_exit',
      'recovery',
      'residue_guard',
      'mode',
      'config_matches',
      'deletes_blocked',
      'path_usage',
      'byte_usage',
    ])
  )
    return null;
  const batch = value.last_batch;
  const paths = usage(value.path_usage),
    content = usage(value.byte_usage);
  if (paths === undefined || content === undefined) return null;
  if (
    batch !== null &&
    (!object(batch) ||
      !keys(batch, ['sequence', 'closed_at']) ||
      !decimal(batch.sequence) ||
      !optional(batch.closed_at, timestamp))
  )
    return null;
  if (
    ![value.fused, value.quiescence_valid, value.config_matches].every((item) =>
      optional(item, (v) => typeof v === 'boolean'),
    ) ||
    ![
      value.unknown_jobs,
      value.conflict_copies,
      value.mirror_bytes,
      value.deletes_blocked,
    ].every((item) => optional(item, decimal))
  )
    return null;
  if (
    !optional(value.last_exit, (v) =>
      oneOf(v, [
        'clean',
        'non_zero',
        'signal',
        'deadline_exceeded',
        'owner_stopped',
        'unconfirmed',
      ]),
    ) ||
    !optional(value.recovery, (v) =>
      oneOf(v, ['idle', 'forced_mirror_remote', 'backoff']),
    ) ||
    !optional(value.residue_guard, (v) => oneOf(v, ['clear', 'blocked'])) ||
    !optional(value.mode, (v) => oneOf(v, ['mirror-remote', 'bidirectional']))
  )
    return null;
  return {
    lastBatch:
      batch === null
        ? null
        : {
            sequence: (batch as Record<string, unknown>).sequence as string,
            closedAt: (batch as Record<string, unknown>).closed_at as
              string | null,
          },
    fused: value.fused as boolean | null,
    unknownJobs: value.unknown_jobs as string | null,
    quiescenceValid: value.quiescence_valid as boolean | null,
    conflictCopies: value.conflict_copies as string | null,
    mirrorBytes: value.mirror_bytes as string | null,
    lastExit: value.last_exit as BridgeHealth['lastExit'],
    recovery: value.recovery as BridgeHealth['recovery'],
    residueGuard: value.residue_guard as BridgeHealth['residueGuard'],
    mode: value.mode as BridgeHealth['mode'],
    configMatches: value.config_matches as boolean | null,
    deletesBlocked: value.deletes_blocked as string | null,
    pathUsage: paths,
    byteUsage: content,
  };
}
/** 一项用量（路径条数或比对内容字节数）到了上限的八成（含用满、超过）：后端据此降级（BUG-047），与 `bridge.rs` 同一口径。 */
export function nearBudget(value: BudgetUsage): boolean {
  return BigInt(value.used) * 5n >= BigInt(value.budget) * 4n;
}
/**
 * “正常”必须与每项证据一致：十一项必备证据都已取得，而且没有任何一条正证据
 * （与后端 `bridge.rs::judge` 同一口径，T-223）。其他状态仍保留后端的结论：
 * 后端有正证据就报降级、不等字段齐，明细不全时这里也不把降级改成未知。
 */
export function bridgeCanBeNormal(value: BridgeHealth): boolean {
  return (
    value.lastBatch !== null &&
    value.lastBatch.closedAt !== null &&
    value.fused === false &&
    value.unknownJobs === '0' &&
    value.quiescenceValid === true &&
    value.conflictCopies !== null &&
    value.mirrorBytes !== null &&
    value.lastExit === 'clean' &&
    value.recovery === 'idle' &&
    value.residueGuard === 'clear' &&
    value.mode !== null &&
    value.configMatches === true &&
    // 没证明过不单独挡正常（与后端同一口径）；有被挡的删除就不能是正常。
    (value.deletesBlocked ?? '0') === '0' &&
    // 路径用量、比对内容用量同理：没算到过不挡；到八成或超上限就不能是正常。
    (value.pathUsage === null || !nearBudget(value.pathUsage)) &&
    (value.byteUsage === null || !nearBudget(value.byteUsage))
  );
}
export function parseManualQuota(value: unknown): ManualQuota | null {
  if (
    !object(value) ||
    !keys(value, [
      'account_limit_bytes',
      'used_bytes_manual',
      'recorded_at',
      'as_of',
      'warning_percent',
      'max_age_seconds',
      'state',
      'reason',
    ])
  )
    return null;
  if (
    !decimal(value.account_limit_bytes) ||
    !decimal(value.used_bytes_manual) ||
    !optional(value.recorded_at, timestamp) ||
    !optional(value.as_of, timestamp) ||
    value.warning_percent !== 70 ||
    value.max_age_seconds !== 86400
  )
    return null;
  if (
    !oneOf(value.state, ['normal', 'degraded', 'unknown']) ||
    !oneOf(value.reason, [
      'within',
      'warning',
      'exceeded',
      'clock',
      'stale',
      'invalid',
    ])
  )
    return null;
  const limit = BigInt(value.account_limit_bytes),
    used = BigInt(value.used_bytes_manual);
  const age =
    typeof value.recorded_at === 'string' && typeof value.as_of === 'string'
      ? (Date.parse(value.as_of) - Date.parse(value.recorded_at)) / 1000
      : null;
  const expected =
    limit === 0n
      ? 'invalid'
      : age === null || age < 0
        ? 'clock'
        : age > 86400
          ? 'stale'
          : used > limit
            ? 'exceeded'
            : used * 100n >= limit * 70n
              ? 'warning'
              : 'within';
  const state =
    expected === 'within'
      ? 'normal'
      : expected === 'warning' || expected === 'exceeded'
        ? 'degraded'
        : 'unknown';
  if (expected !== value.reason || state !== value.state) return null;
  return {
    accountLimitBytes: value.account_limit_bytes,
    usedBytesManual: value.used_bytes_manual,
    recordedAt: value.recorded_at as string | null,
    asOf: value.as_of as string | null,
    state,
    reason: expected,
  };
}

const FAILURE_REASONS = [
  'barrier_not_established',
  'source_changed_before_seal',
  'out_of_space',
  'upload_rejected',
  'remote_verification_failed',
  'final_manifest_commit_failed',
  'target_unverified',
  'source_expired',
] as const;
const UNVERIFIED_REASONS = [
  'no_real_repository',
  'target_unreachable',
  'immutability_unprovable',
] as const;
export type BackupFailureReason = (typeof FAILURE_REASONS)[number];
export type BackupUnverifiedReason = (typeof UNVERIFIED_REASONS)[number];
export type BackupVerdict =
  | 'healthy'
  | 'stuck'
  | 'overdue'
  | 'latest_failed'
  | 'clock_behind'
  | 'no_success_yet';
export type BackupFailure =
  | { result: 'failed'; reason: BackupFailureReason }
  | { result: 'unverified'; reason: BackupUnverifiedReason }
  | { result: 'unrecognized'; reason: null };

/** 备份作业明细：和「备份作业」一节出自同一次读账本；序号和秒数保留十进制字符串。 */
export interface BackupJobs {
  state: 'normal' | 'degraded' | 'unknown';
  verdict: BackupVerdict;
  asOf: string | null;
  lastSuccess: { sequence: string; at: string | null } | null;
  outstanding: {
    sequence: string;
    since: string | null;
    admitted: boolean;
    stuck: boolean | null;
  } | null;
  overdue: boolean | null;
  overdueSince: string | null;
  latestFailed: boolean;
  lastFailure: ({ sequence: string; at: string | null } & BackupFailure) | null;
  stuckAfterSeconds: string;
  overdueAfterSeconds: string;
}

const VERDICT_STATE: Readonly<Record<BackupVerdict, BackupJobs['state']>> = {
  healthy: 'normal',
  stuck: 'degraded',
  overdue: 'degraded',
  latest_failed: 'degraded',
  clock_behind: 'unknown',
  no_success_yet: 'unknown',
};
const isBool = (value: unknown) => typeof value === 'boolean';

/** 和后端 `BackupJobs::verdict` 同一个顺序：已知的问题排在判断不了前面。 */
function expectedVerdict(
  stuck: boolean | null,
  overdue: boolean | null,
  latestFailed: boolean,
  hasSuccess: boolean,
): BackupVerdict {
  if (stuck === true) return 'stuck';
  if (overdue === true) return 'overdue';
  if (latestFailed) return 'latest_failed';
  if (stuck === null || overdue === null) return 'clock_behind';
  return hasSuccess ? 'healthy' : 'no_success_yet';
}

function parseFailure(value: unknown): BackupJobs['lastFailure'] | undefined {
  if (value === null) return null;
  if (
    !object(value) ||
    !keys(value, ['sequence', 'at', 'result', 'reason']) ||
    !decimal(value.sequence) ||
    !optional(value.at, timestamp)
  )
    return undefined;
  const mark = {
    sequence: value.sequence as string,
    at: value.at as string | null,
  };
  if (value.result === 'failed' && oneOf(value.reason, FAILURE_REASONS))
    return { ...mark, result: 'failed', reason: value.reason };
  if (value.result === 'unverified' && oneOf(value.reason, UNVERIFIED_REASONS))
    return { ...mark, result: 'unverified', reason: value.reason };
  if (value.result === 'unrecognized' && value.reason === null)
    return { ...mark, result: 'unrecognized', reason: null };
  return undefined;
}

/**
 * 备份作业明细的封闭解析。形状不对、原因认不出、结论和明细对不上，整份都不认：
 * 调用方据此把「备份作业」一节降为未知，不让一份自相矛盾的明细撑起「正常」。
 */
export function parseBackupJobs(value: unknown): BackupJobs | null {
  if (
    !object(value) ||
    !keys(value, [
      'state',
      'verdict',
      'as_of',
      'last_success',
      'outstanding',
      'overdue',
      'overdue_since',
      'latest_failed',
      'last_failure',
      'stuck_after_seconds',
      'overdue_after_seconds',
    ]) ||
    !oneOf(value.verdict, Object.keys(VERDICT_STATE) as BackupVerdict[]) ||
    value.state !== VERDICT_STATE[value.verdict] ||
    !optional(value.as_of, timestamp) ||
    !optional(value.overdue, isBool) ||
    !optional(value.overdue_since, timestamp) ||
    !isBool(value.latest_failed) ||
    !decimal(value.stuck_after_seconds) ||
    !decimal(value.overdue_after_seconds)
  )
    return null;
  const success = value.last_success;
  if (
    success !== null &&
    (!object(success) ||
      !keys(success, ['sequence', 'at']) ||
      !decimal(success.sequence) ||
      !optional(success.at, timestamp))
  )
    return null;
  const open = value.outstanding;
  if (
    open !== null &&
    (!object(open) ||
      !keys(open, ['sequence', 'since', 'admitted', 'stuck']) ||
      !decimal(open.sequence) ||
      !optional(open.since, timestamp) ||
      !isBool(open.admitted) ||
      !optional(open.stuck, isBool))
  )
    return null;
  const failure = parseFailure(value.last_failure);
  if (failure === undefined) return null;
  const verdict = value.verdict as BackupVerdict;
  const latestFailed = value.latest_failed as boolean;
  const overdue = value.overdue as boolean | null;
  const held = open as Record<string, unknown> | null;
  const stuck = held === null ? false : (held.stuck as boolean | null);
  if (
    (latestFailed && failure === null) ||
    verdict !== expectedVerdict(stuck, overdue, latestFailed, success !== null)
  )
    return null;
  const done = success as Record<string, unknown> | null;
  return {
    state: VERDICT_STATE[verdict],
    verdict,
    asOf: value.as_of as string | null,
    lastSuccess:
      done === null
        ? null
        : { sequence: done.sequence as string, at: done.at as string | null },
    outstanding:
      held === null
        ? null
        : {
            sequence: held.sequence as string,
            since: held.since as string | null,
            admitted: held.admitted as boolean,
            stuck: held.stuck as boolean | null,
          },
    overdue,
    overdueSince: value.overdue_since as string | null,
    latestFailed,
    lastFailure: failure,
    stuckAfterSeconds: value.stuck_after_seconds as string,
    overdueAfterSeconds: value.overdue_after_seconds as string,
  };
}
