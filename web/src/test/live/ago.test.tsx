/*
 * 「多久以前」的说法和自己走字的组件（T-196，live/ago.ts、live/Ago.tsx）。
 */
import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Ago } from '../../live/Ago';
import { agoLabel, agoValidFor } from '../../live/ago';

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

describe('agoLabel', () => {
  it.each([
    [0, '刚刚'],
    [4_999, '刚刚'],
    [5_000, '5 秒前'],
    [9_999, '5 秒前'],
    [10_000, '10 秒前'],
    [59_999, '55 秒前'],
    [MINUTE, '1 分钟前'],
    [2 * MINUTE - 1, '1 分钟前'],
    [59 * MINUTE + 59_999, '59 分钟前'],
    [HOUR, '1 小时前'],
    [DAY - 1, '23 小时前'],
    [DAY, '1 天前'],
    [3 * DAY + HOUR, '3 天前'],
  ])('%i 毫秒 → %s', (elapsed, label) => {
    expect(agoLabel(elapsed)).toBe(label);
  });

  it('本机时钟往回拨算出负数时按 0 算', () => {
    expect(agoLabel(-30_000)).toBe('刚刚');
    expect(agoValidFor(-30_000)).toBe(5_000);
  });
});

describe('agoValidFor', () => {
  it.each([
    [0, 5_000],
    [4_999, 1],
    [5_000, 5_000],
    [57_000, 3_000],
    [MINUTE, MINUTE],
    [MINUTE + 1, MINUTE - 1],
    [HOUR - 1, 1],
    [HOUR, HOUR],
    [DAY + 5, DAY - 5],
  ])('%i 毫秒时这句说法还能用 %i 毫秒', (elapsed, valid) => {
    expect(agoValidFor(elapsed)).toBe(valid);
  });

  it('到点之后说法一定变了', () => {
    for (const elapsed of [0, 4_000, 31_000, 59_500, 7 * MINUTE + 3, HOUR - 5])
      expect(agoLabel(elapsed + agoValidFor(elapsed))).not.toBe(
        agoLabel(elapsed),
      );
  });
});

describe('Ago', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('自己走字，不靠上层重画；时刻换了就从「刚刚」重新走', async () => {
    vi.useFakeTimers();
    const start = new Date(2026, 10, 12, 21, 7, 35).getTime();
    vi.setSystemTime(start);
    const { rerender } = render(<Ago at={start} />);
    const stamp = screen.getByText('刚刚');
    expect(stamp.tagName).toBe('TIME');
    expect(stamp).toHaveAttribute('title', '21:07:35');
    expect(stamp).toHaveAttribute('datetime', new Date(start).toISOString());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_100);
    });
    expect(stamp).toHaveTextContent('5 秒前');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50_000);
    });
    expect(stamp).toHaveTextContent('55 秒前');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(stamp).toHaveTextContent('1 分钟前');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3 * MINUTE);
    });
    expect(stamp).toHaveTextContent('4 分钟前');

    rerender(<Ago at={Date.now()} />);
    expect(stamp).toHaveTextContent('刚刚');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_100);
    });
    expect(stamp).toHaveTextContent('10 秒前');
  });

  it('页面切回来时立刻对一次表', async () => {
    vi.useFakeTimers();
    const start = new Date(2026, 10, 12, 9, 0, 0).getTime();
    vi.setSystemTime(start);
    render(<Ago at={start} />);
    // 后台标签页的定时器被拖住：时钟走了 3 分钟，定时器一个都没触发。
    vi.setSystemTime(start + 3 * MINUTE);
    expect(screen.getByText('刚刚')).toBeInTheDocument();

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(screen.getByText('3 分钟前')).toBeInTheDocument();
  });
});
