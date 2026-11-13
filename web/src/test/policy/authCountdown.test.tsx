import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AuthCountdown } from '../../policy/AuthCountdown';
import type { AuthWindow } from '../../policy/recentAuth';

/** 测试自己拨的单调时钟：和定时器一起往前走。 */
function clock(start: number) {
  let value = start;
  return {
    now: () => value,
    /** 时钟和定时器一起走 `ms`。 */
    pass(ms: number) {
      value += ms;
      act(() => {
        vi.advanceTimersByTime(ms);
      });
    },
  };
}

function mount(span: AuthWindow, time: ReturnType<typeof clock>) {
  const onExpire = vi.fn<() => void>();
  const view = render(
    <AuthCountdown
      className="note"
      span={span}
      now={time.now}
      onExpire={onExpire}
    />,
  );
  return {
    onExpire,
    line: () => view.container.querySelector('p'),
    rerender: (next: AuthWindow) =>
      view.rerender(
        <AuthCountdown
          className="note"
          span={next}
          now={time.now}
          onExpire={onExpire}
        />,
      ),
    unmount: view.unmount,
  };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('「二次认证还有效」那一行', () => {
  it('写明还剩多久，并说清这只是告知：到时候要不要输由服务端说了算', () => {
    const time = clock(5_000);
    const { line } = mount({ remainingMs: 252_000, at: 5_000 }, time);
    expect(line()).toHaveClass('note');
    expect(line()).toHaveTextContent(
      '二次认证还有效 4 分 12 秒：这段时间里再批准，一般不用重新输入验证码（以服务端为准）。',
    );
  });

  it('每秒走一次字，照给的单调时钟算', () => {
    const time = clock(5_000);
    const { line } = mount({ remainingMs: 62_000, at: 5_000 }, time);
    expect(line()).toHaveTextContent('还有效 1 分 2 秒');
    time.pass(1_000);
    expect(line()).toHaveTextContent('还有效 1 分 1 秒');
    time.pass(2_000);
    expect(line()).toHaveTextContent('还有效 59 秒');
  });

  it('读到之后才挂上来：从读到那一刻算，已经过去的那一段照样扣掉', () => {
    // 读到时还剩 30 秒，画出来的时候已经过了 12 秒。
    const time = clock(20_000);
    const { line } = mount({ remainingMs: 30_000, at: 8_000 }, time);
    expect(line()).toHaveTextContent('还有效 18 秒');
  });

  it('到点：这一行撤掉，告诉上面一次，之后不再空转', () => {
    const time = clock(0);
    const { line, onExpire } = mount({ remainingMs: 3_000, at: 0 }, time);
    time.pass(2_000);
    expect(line()).toHaveTextContent('还有效 1 秒');
    expect(onExpire).not.toHaveBeenCalled();
    time.pass(1_000);
    expect(line()).toBeNull();
    expect(onExpire).toHaveBeenCalledTimes(1);
    time.pass(10_000);
    expect(onExpire).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('挂上来时已经到期：什么都不画，当即告诉上面一次', () => {
    const time = clock(9_000);
    const { line, onExpire } = mount({ remainingMs: 3_000, at: 0 }, time);
    expect(line()).toBeNull();
    expect(onExpire).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('换了一份新读到的窗口：照新的重新走', () => {
    const time = clock(0);
    const { line, rerender, onExpire } = mount(
      { remainingMs: 4_000, at: 0 },
      time,
    );
    time.pass(3_000);
    expect(line()).toHaveTextContent('还有效 1 秒');
    rerender({ remainingMs: 300_000, at: 3_000 });
    expect(line()).toHaveTextContent('还有效 5 分');
    time.pass(1_000);
    expect(line()).toHaveTextContent('还有效 4 分 59 秒');
    expect(onExpire).not.toHaveBeenCalled();
  });

  it('不是播报区：每秒都在变的字不让读屏每秒念一遍', () => {
    const time = clock(0);
    const { line } = mount({ remainingMs: 60_000, at: 0 }, time);
    expect(line()).not.toHaveAttribute('role');
    expect(line()).not.toHaveAttribute('aria-live');
  });

  it('卸载就停表', () => {
    const time = clock(0);
    const { unmount, onExpire } = mount({ remainingMs: 2_000, at: 0 }, time);
    unmount();
    expect(vi.getTimerCount()).toBe(0);
    time.pass(5_000);
    expect(onExpire).not.toHaveBeenCalled();
  });
});
