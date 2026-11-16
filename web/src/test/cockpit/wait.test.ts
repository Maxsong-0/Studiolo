import { describe, expect, it } from 'vitest';
import { DAY_MS, HOUR_MS, MINUTE_MS } from '../../cockpit/model';
import {
  WAIT_MARKS,
  agoLabel,
  waitFraction,
  waitLabel,
} from '../../cockpit/wait';

describe('等待时长的对数刻度', () => {
  it('三个锚点正好落在三成、一半、九成', () => {
    expect(WAIT_MARKS.map((mark) => [mark.ms, mark.at, mark.label])).toEqual([
      [10 * MINUTE_MS, 0.3, '10分'],
      [HOUR_MS, 0.5, '1时'],
      [DAY_MS, 0.9, '1天'],
    ]);
    for (const mark of WAIT_MARKS)
      expect(waitFraction(mark.ms)).toBeCloseTo(mark.at, 10);
  });

  it('锚点之间按对数插值：两个锚点的几何中点落在两个位置的正中', () => {
    expect(waitFraction(Math.sqrt(10 * MINUTE_MS * HOUR_MS))).toBeCloseTo(
      0.4,
      10,
    );
    expect(waitFraction(Math.sqrt(HOUR_MS * DAY_MS))).toBeCloseTo(0.7, 10);
  });

  it('越久越长，两头收在 0 和 1', () => {
    const samples = [
      1,
      30_000,
      MINUTE_MS,
      5 * MINUTE_MS,
      10 * MINUTE_MS,
      30 * MINUTE_MS,
      HOUR_MS,
      6 * HOUR_MS,
      DAY_MS,
      2 * DAY_MS,
      30 * DAY_MS,
    ];
    const fractions = samples.map(waitFraction);
    for (let index = 1; index < fractions.length; index += 1)
      expect(fractions[index]).toBeGreaterThanOrEqual(fractions[index - 1]);
    expect(fractions[0]).toBe(0);
    expect(fractions[fractions.length - 1]).toBe(1);
    // 1 分钟已经看得见一小段，9 分钟和 1 天差得出长短。
    expect(waitFraction(MINUTE_MS)).toBeGreaterThan(0.02);
    expect(waitFraction(DAY_MS) - waitFraction(9 * MINUTE_MS)).toBeGreaterThan(
      0.5,
    );
  });

  it('不是有限正数时不画', () => {
    for (const value of [0, -5, Number.NaN, Number.POSITIVE_INFINITY])
      expect(waitFraction(value)).toBe(0);
  });
});

describe('等待时长的文字', () => {
  it.each<[number, string]>([
    [0, '不到 1 分'],
    [-3 * MINUTE_MS, '不到 1 分'],
    [MINUTE_MS - 1, '不到 1 分'],
    [MINUTE_MS, '1 分'],
    [47 * MINUTE_MS + 59_000, '47 分'],
    [HOUR_MS, '1 时'],
    [2 * HOUR_MS + 14 * MINUTE_MS, '2 时 14 分'],
    [DAY_MS - 1, '23 时 59 分'],
    [DAY_MS, '1 天'],
    [DAY_MS + 3 * HOUR_MS + 20 * MINUTE_MS, '1 天 3 时'],
    [9 * DAY_MS, '9 天'],
  ])('等了 %d 毫秒写成「%s」：只取最大的两个单位，零头不写', (ms, label) => {
    expect(waitLabel(ms)).toBe(label);
  });

  it.each<[number, string]>([
    [0, '不到 1 分前'],
    [-MINUTE_MS, '不到 1 分前'],
    [12 * MINUTE_MS + 30_000, '12 分前'],
    [3 * HOUR_MS + 59 * MINUTE_MS, '3 时前'],
    [2 * DAY_MS + 23 * HOUR_MS, '2 天前'],
  ])('%d 毫秒之前写成「%s」：只取最大的一个单位', (ms, label) => {
    expect(agoLabel(ms)).toBe(label);
  });
});
