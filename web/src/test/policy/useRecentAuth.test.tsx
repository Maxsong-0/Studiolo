import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { installFactorRuntime, type FactorRuntime } from '../../factorApi';
import type { FactorState } from '../../factorModel';
import type { RecentAuth } from '../../policy/recentAuth';
import { monotonicNow, useRecentAuth } from '../../policy/useRecentAuth';

type ReadState = FactorRuntime['state'];

let uninstall: (() => void) | undefined;
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
});

/** 装一个假的二次认证入口；`state` 是读因子状态的那一个函数。 */
function install(state: ReadState) {
  const submit = vi.fn<FactorRuntime['submit']>();
  uninstall = installFactorRuntime({ state, submit });
  return submit;
}

function factors(expires: number | null, observed = 1_000_000): FactorState {
  return {
    observed_at_ms: observed,
    recent_authentication:
      expires === null
        ? null
        : { verified_at_ms: observed - 1_000, expires_at_ms: expires },
    recovery_candidate_available: false,
    pending: false,
  };
}

/** 一个由测试决定什么时候给结果的读取。 */
function deferred() {
  let resolve!: (value: FactorState | null) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<FactorState | null>((ok, fail) => {
    resolve = ok;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const signal = () => new AbortController().signal;

describe('近期二次认证的那一份读取', () => {
  it('没读过是「不知道」；读取不在挂载时自己发', () => {
    const state = vi.fn<ReadState>();
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 7));
    expect(result.current.state).toEqual({ kind: 'unknown' });
    expect(state).not.toHaveBeenCalled();
  });

  it('读到近期认证：记下还剩多久和读到时的时钟读数，结论同时交回调用方', async () => {
    const state = vi.fn<ReadState>(async () => factors(1_240_000));
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 7));
    const caller = signal();
    let answer: RecentAuth | undefined;
    await act(async () => {
      answer = await result.current.read(caller);
    });
    const expected: RecentAuth = {
      kind: 'valid',
      window: { remainingMs: 240_000, at: 7 },
    };
    expect(answer).toEqual(expected);
    expect(result.current.state).toEqual(expected);
    // 调用方的中止信号原样交给会话那一层。
    expect(state).toHaveBeenCalledTimes(1);
    expect(state).toHaveBeenCalledWith(caller);
    expect(result.current.now()).toBe(7);
  });

  it('没有近期认证、本入口没有验证码路径：照实记下', async () => {
    const state = vi
      .fn<ReadState>()
      .mockResolvedValueOnce(factors(null))
      .mockResolvedValueOnce(null);
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 0));
    await act(async () => {
      await result.current.read(signal());
    });
    expect(result.current.state).toEqual({ kind: 'none' });
    await act(async () => {
      await result.current.read(signal());
    });
    expect(result.current.state).toEqual({ kind: 'no-path' });
  });

  it('本页没有受控会话：不发请求，记「没有会话」', async () => {
    const { result } = renderHook(() => useRecentAuth(() => 0));
    let answer: RecentAuth | undefined;
    await act(async () => {
      answer = await result.current.read(signal());
    });
    expect(answer).toEqual({ kind: 'no-session' });
    expect(result.current.state).toEqual({ kind: 'no-session' });
  });

  it('读取出错：回到「不知道」，不留着上一次的「还有效」冒充现状', async () => {
    const state = vi
      .fn<ReadState>()
      .mockResolvedValueOnce(factors(1_240_000))
      .mockRejectedValueOnce(new Error('读取不可用'));
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 0));
    await act(async () => {
      await result.current.read(signal());
    });
    expect(result.current.state.kind).toBe('valid');
    let answer: RecentAuth | undefined;
    await act(async () => {
      answer = await result.current.read(signal());
    });
    expect(answer).toEqual({ kind: 'unknown' });
    expect(result.current.state).toEqual({ kind: 'unknown' });
  });

  it('只认最后发出的那一次：先发后到的旧结论不盖新的，也不交回调用方', async () => {
    const first = deferred();
    const second = deferred();
    const state = vi
      .fn<ReadState>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 3));
    let early: Promise<RecentAuth> | undefined;
    let late: Promise<RecentAuth> | undefined;
    act(() => {
      early = result.current.read(signal());
      late = result.current.read(signal());
    });
    await act(async () => {
      second.resolve(factors(null));
      await late;
    });
    expect(result.current.state).toEqual({ kind: 'none' });
    await act(async () => {
      first.resolve(factors(1_300_000));
      await early;
    });
    await expect(early).resolves.toEqual({ kind: 'unknown' });
    expect(result.current.state).toEqual({ kind: 'none' });
  });

  it('调用方不要了（信号已中止）：结论不记，交回「不知道」', async () => {
    const pending = deferred();
    install(vi.fn<ReadState>(() => pending.promise));
    const { result } = renderHook(() => useRecentAuth(() => 0));
    const controller = new AbortController();
    let answer: Promise<RecentAuth> | undefined;
    act(() => {
      answer = result.current.read(controller.signal);
    });
    controller.abort();
    await act(async () => {
      pending.resolve(factors(1_300_000));
      await answer;
    });
    await expect(answer).resolves.toEqual({ kind: 'unknown' });
    expect(result.current.state).toEqual({ kind: 'unknown' });
  });

  it('倒计时走到头：「还有效」改成「没有」；别的结论不动', async () => {
    const state = vi
      .fn<ReadState>()
      .mockResolvedValueOnce(factors(1_010_000))
      .mockResolvedValueOnce(null);
    install(state);
    const { result } = renderHook(() => useRecentAuth(() => 0));
    await act(async () => {
      await result.current.read(signal());
    });
    expect(result.current.state.kind).toBe('valid');
    act(() => result.current.expire());
    expect(result.current.state).toEqual({ kind: 'none' });
    await act(async () => {
      await result.current.read(signal());
    });
    act(() => result.current.expire());
    expect(result.current.state).toEqual({ kind: 'no-path' });
  });

  it('只读：从不提交验证码', async () => {
    const submit = install(vi.fn<ReadState>(async () => factors(1_300_000)));
    const { result } = renderHook(() => useRecentAuth(() => 0));
    await act(async () => {
      await result.current.read(signal());
    });
    act(() => result.current.expire());
    expect(submit).not.toHaveBeenCalled();
  });

  it('状态没变时交出去的是同一个对象（下游不白白重画）', async () => {
    install(vi.fn<ReadState>(async () => factors(null)));
    const { result, rerender } = renderHook(() => useRecentAuth());
    const before = result.current;
    rerender();
    expect(result.current).toBe(before);
    // 缺省用本机的单调时钟。
    expect(result.current.now).toBe(monotonicNow);
  });
});
