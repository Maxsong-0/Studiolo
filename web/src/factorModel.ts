export type RecentAuthentication = {
  verified_at_ms: number;
  expires_at_ms: number;
};
export type FactorState = {
  observed_at_ms: number;
  recent_authentication: RecentAuthentication | null;
  recovery_candidate_available: boolean;
  pending: boolean;
};
export type FactorAction =
  | { kind: 'step-up'; code: string }
  | { kind: 'codes' }
  | { kind: 'recover'; recovery_code: string; code: string };
export type FactorSuccess =
  | { kind: 'step-up'; recent: RecentAuthentication }
  | { kind: 'codes'; codes: string[] }
  | { kind: 'recovered' };
export type FactorOutcome =
  | FactorSuccess
  | { kind: 'failed'; message: string; status?: number }
  | { kind: 'stopped' };

function invalid(): never {
  throw new Error('安全状态响应格式无效，请重新核对。');
}
function exact(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  const object = value as Record<string, unknown>;
  if (
    Object.keys(object).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(object, key))
  )
    invalid();
  return object;
}
function timestamp(value: unknown): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) invalid();
  return value as number;
}
export function recentAuthentication(value: unknown): RecentAuthentication {
  const recent = exact(value, ['verified_at_ms', 'expires_at_ms']);
  const verified_at_ms = timestamp(recent.verified_at_ms);
  const expires_at_ms = timestamp(recent.expires_at_ms);
  if (
    expires_at_ms <= verified_at_ms ||
    expires_at_ms - verified_at_ms > 300_000
  )
    invalid();
  return { verified_at_ms, expires_at_ms };
}
export function factorState(value: unknown): FactorState | null {
  const item = exact(value, ['factors']).factors;
  if (item === null) return null;
  const state = exact(item, [
    'observed_at_ms',
    'recent_authentication',
    'recovery_candidate_available',
    'pending',
  ]);
  const observed_at_ms = timestamp(state.observed_at_ms);
  const recent =
    state.recent_authentication === null
      ? null
      : recentAuthentication(state.recent_authentication);
  if (
    typeof state.recovery_candidate_available !== 'boolean' ||
    typeof state.pending !== 'boolean' ||
    (recent &&
      (recent.verified_at_ms > observed_at_ms ||
        recent.expires_at_ms <= observed_at_ms))
  )
    invalid();
  return {
    observed_at_ms,
    recent_authentication: recent,
    recovery_candidate_available: state.recovery_candidate_available,
    pending: state.pending,
  };
}
export function validOtp(code: string) {
  return /^[0-9]{6}$/.test(code);
}
export function validRecoveryCode(code: string) {
  if (!/^r1\.[0-7][0-9a-f]{15}\.0[0-9]\.[0-9a-f]{32}$/.test(code)) return false;
  return code.slice(3, 19) !== '0000000000000000';
}
export function recoveryCodes(value: unknown): string[] {
  const codes = exact(value, ['codes']).codes;
  if (
    !Array.isArray(codes) ||
    codes.length !== 10 ||
    codes.some((code) => typeof code !== 'string' || !validRecoveryCode(code))
  )
    invalid();
  const items = codes as string[];
  if (
    new Set(items.map((code) => code.slice(20, 22))).size !== 10 ||
    new Set(items.map((code) => code.slice(3, 19))).size !== 1 ||
    new Set(items.map((code) => code.slice(23))).size !== 10
  )
    invalid();
  return items;
}
export function factorFailure(status: number): string {
  if (status === 403)
    return '验证或请求未通过。输入可能已被消费，请先核对安全状态，再使用新的验证码。';
  if (status === 409)
    return '认证操作尚待处理，或当前没有可用的恢复候选。请先核对安全状态。';
  if (status === 429)
    return '认证请求已达限制。请稍后核对状态，不要连续提交验证码。';
  return '安全操作未完成。请核对安全状态后再决定下一步。';
}
export function factorUncertain(kind: FactorAction['kind']): string {
  if (kind === 'codes')
    return '恢复码可能已经换发，但本次原码无法重读。核对状态不能确认交付；再次生成是独立的新操作，会使之前的恢复码失效。';
  if (kind === 'recover')
    return '验证器可能已经恢复，原会话可能已失效。请核对当前状态，不要重复使用刚才的恢复码或验证码。';
  return '二次认证可能已生效，验证码也可能已被消费。请先核对状态，不要重复提交刚才的验证码。';
}
