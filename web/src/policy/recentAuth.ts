/**
 * 近期二次认证还剩多久（T-200，纯函数）。只做显示：要不要重新认证始终由服务端在发布那一刻裁定，
 * 这里的数不参与任何判断——显示「还有效」而服务端说要认证，照样弹认证窗口。
 *
 * 窗口多长不在前端算：服务端的因子状态同时给了「观察时刻」和「到期时刻」（都是服务端的钟），
 * 两者相减就是读到那一刻还剩多少；之后按本机的单调时钟往下走，不看本机的墙上时间
 * （本机时间不准、被人改过，都不影响）。响应在路上花的那点时间没法扣，所以显示的数会比实际略多一点。
 */
import type { FactorState } from '../factorModel';

/** 读到的那一刻还剩多少毫秒，以及读到时本机单调时钟的读数。 */
export interface AuthWindow {
  remainingMs: number;
  /** `performance.now()` 那一类单调时钟的读数。 */
  at: number;
}

/** 一次读取的结论。 */
export type RecentAuth =
  /** 没读过，或者没读成：不知道，什么都不写。 */
  | { kind: 'unknown' }
  /** 本页没有受控会话，没法就地认证。 */
  | { kind: 'no-session' }
  /** 本入口没有一次性验证码路径：这里发布不了。 */
  | { kind: 'no-path' }
  /** 没有还有效的近期认证：下一次发布要输入验证码。 */
  | { kind: 'none' }
  | { kind: 'valid'; window: AuthWindow };

export const UNKNOWN_AUTH: RecentAuth = { kind: 'unknown' };

/** 服务端的因子状态 → 读到那一刻的近期认证窗口；没有、或者已经到期，返回 null。 */
export function authWindow(state: FactorState, at: number): AuthWindow | null {
  const recent = state.recent_authentication;
  if (!recent) return null;
  const remainingMs = recent.expires_at_ms - state.observed_at_ms;
  return remainingMs > 0 ? { remainingMs, at } : null;
}

/** 因子状态（`null` 表示本入口没有验证码路径）→ 一次读取的结论。 */
export function recentAuthOf(
  state: FactorState | null,
  at: number,
): RecentAuth {
  if (state === null) return { kind: 'no-path' };
  const window = authWindow(state, at);
  return window ? { kind: 'valid', window } : { kind: 'none' };
}

/** 现在还剩多少毫秒；到期之后是 0。单调时钟倒着走（不该发生）时按没过时间算。 */
export function remainingMs(window: AuthWindow, now: number): number {
  return Math.max(0, window.remainingMs - Math.max(0, now - window.at));
}

/** 「4 分 12 秒」「48 秒」：向下取整到秒，不到一秒写「不到 1 秒」。 */
export function remainingLabel(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds <= 0) return '不到 1 秒';
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes === 0) return `${rest} 秒`;
  return rest === 0 ? `${minutes} 分` : `${minutes} 分 ${rest} 秒`;
}

/**
 * 发布预览里二次认证那一句：知道近期认证还有效就照实说（确认后多半不用再输），否则返回 undefined，
 * 让预览用缺省那一句。无论哪种，到时候要不要输由服务端说了算。
 */
export function stepUpNote(auth: RecentAuth, now: number): string | undefined {
  if (auth.kind !== 'valid') return undefined;
  const left = remainingMs(auth.window, now);
  if (left <= 0) return undefined;
  return `二次认证还有效约 ${remainingLabel(left)}（以服务端为准）：确认后直接发布；要是到期了，会弹出窗口请你输入一次性验证码，通过后才会发布。`;
}
