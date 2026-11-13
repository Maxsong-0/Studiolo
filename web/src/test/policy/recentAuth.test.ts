import { describe, expect, it } from 'vitest';
import type { FactorState } from '../../factorModel';
import {
  UNKNOWN_AUTH,
  authWindow,
  recentAuthOf,
  remainingLabel,
  remainingMs,
  stepUpNote,
  type RecentAuth,
} from '../../policy/recentAuth';

/** 服务端在 `observed` 这一刻给出的因子状态；`expires` 是近期认证的到期时刻（null＝没有近期认证）。 */
function factors(expires: number | null, observed = 1_000_000): FactorState {
  return {
    observed_at_ms: observed,
    recent_authentication:
      expires === null
        ? null
        : { verified_at_ms: observed - 60_000, expires_at_ms: expires },
    recovery_candidate_available: false,
    pending: false,
  };
}

const valid = (remaining: number, at: number): RecentAuth => ({
  kind: 'valid',
  window: { remainingMs: remaining, at },
});

describe('近期二次认证还剩多久：从服务端的因子状态算', () => {
  it('剩余时间＝服务端的到期时刻减服务端的观察时刻；本机只记读到时的单调时钟读数', () => {
    expect(authWindow(factors(1_240_000), 42)).toEqual({
      remainingMs: 240_000,
      at: 42,
    });
  });

  it('没有近期认证、正好到期、已经过期：都没有窗口', () => {
    expect(authWindow(factors(null), 0)).toBeNull();
    expect(authWindow(factors(1_000_000), 0)).toBeNull();
    expect(authWindow(factors(999_000), 0)).toBeNull();
  });

  it('一次读取的结论：没有验证码路径、没有近期认证、还有效', () => {
    expect(recentAuthOf(null, 7)).toEqual({ kind: 'no-path' });
    expect(recentAuthOf(factors(null), 7)).toEqual({ kind: 'none' });
    expect(recentAuthOf(factors(999_000), 7)).toEqual({ kind: 'none' });
    expect(recentAuthOf(factors(1_012_000), 7)).toEqual(valid(12_000, 7));
  });

  it('没读过是「不知道」', () => {
    expect(UNKNOWN_AUTH).toEqual({ kind: 'unknown' });
  });
});

describe('走字：按本机单调时钟往下减', () => {
  const window = { remainingMs: 10_000, at: 500 };

  it('过了多久减多久；到期之后是 0，不为负', () => {
    expect(remainingMs(window, 500)).toBe(10_000);
    expect(remainingMs(window, 4_500)).toBe(6_000);
    expect(remainingMs(window, 10_500)).toBe(0);
    expect(remainingMs(window, 99_000)).toBe(0);
  });

  it('时钟倒着走（不该发生）：按没过时间算，不会越走越多', () => {
    expect(remainingMs(window, 100)).toBe(10_000);
  });
});

describe('剩余时间的写法', () => {
  it('向下取整到秒：分和秒、只有秒、整分、不到一秒', () => {
    expect(remainingLabel(252_000)).toBe('4 分 12 秒');
    expect(remainingLabel(252_999)).toBe('4 分 12 秒');
    expect(remainingLabel(48_500)).toBe('48 秒');
    expect(remainingLabel(1_000)).toBe('1 秒');
    expect(remainingLabel(240_000)).toBe('4 分');
    expect(remainingLabel(60_000)).toBe('1 分');
    expect(remainingLabel(999)).toBe('不到 1 秒');
    expect(remainingLabel(0)).toBe('不到 1 秒');
  });
});

describe('发布预览里二次认证那一句', () => {
  it('知道近期认证还有效：照实说还剩多久，并写明到期了照样会弹窗口', () => {
    expect(stepUpNote(valid(252_000, 1_000), 1_000)).toBe(
      '二次认证还有效约 4 分 12 秒（以服务端为准）：确认后直接发布；要是到期了，会弹出窗口请你输入一次性验证码，通过后才会发布。',
    );
    // 读到之后又过了 12 秒。
    expect(stepUpNote(valid(252_000, 1_000), 13_000)).toContain(
      '还有效约 4 分（以服务端为准）',
    );
  });

  it('不知道、没有、没有路径、没有会话、已经走到头：不给这一句，预览用缺省的说法', () => {
    const others: RecentAuth[] = [
      UNKNOWN_AUTH,
      { kind: 'none' },
      { kind: 'no-path' },
      { kind: 'no-session' },
    ];
    for (const auth of others) expect(stepUpNote(auth, 0)).toBeUndefined();
    expect(stepUpNote(valid(5_000, 0), 5_000)).toBeUndefined();
    expect(stepUpNote(valid(5_000, 0), 9_000)).toBeUndefined();
  });
});
