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
    ])
  )
    return null;
  const batch = value.last_batch;
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
    ![value.unknown_jobs, value.conflict_copies, value.mirror_bytes].every(
      (item) => optional(item, decimal),
    )
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
  };
}
/** “正常”必须与每项证据一致；其他状态仍保留后端的已知降级原因。 */
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
    value.configMatches === true
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
