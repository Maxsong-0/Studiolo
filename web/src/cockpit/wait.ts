/**
 * 等待时长的两种表示（驾驶舱「等你」队列与「谁在干活」共用）：一行文字，和一条对数刻度条。
 *
 * 刻度三个锚点：10 分在 30%、1 时在 50%、1 天在 90%，锚点之间按时长的对数线性插值；
 * 10 分以下沿用「10 分到 1 时」那一段的斜率（约 41 秒处归零），1 天以上沿用「1 时到 1 天」的斜率（约 2.2 天处到头）。
 * 用对数是为了让「9 分钟」和「1 天」在同一条刻度上都看得出长短：线性刻度下前者只剩一条细线。
 */
import { DAY_MS, HOUR_MS, MINUTE_MS } from './model';

export interface WaitMark {
  ms: number;
  /** 在刻度条上的位置，0 到 1。 */
  at: number;
  label: string;
}

/** 刻度条的三个锚点，从短到长；图例只画这三处。 */
export const WAIT_MARKS: readonly [WaitMark, WaitMark, WaitMark] = [
  { ms: 10 * MINUTE_MS, at: 0.3, label: '10分' },
  { ms: HOUR_MS, at: 0.5, label: '1时' },
  { ms: DAY_MS, at: 0.9, label: '1天' },
];

/** 等了 `ms` 毫秒画到刻度条的几成（0 到 1）；不是有限正数时为 0。 */
export function waitFraction(ms: number): number {
  if (!Number.isFinite(ms) || ms <= 0) return 0;
  const [ten, hour, day] = WAIT_MARKS;
  const [low, high] = ms <= hour.ms ? [ten, hour] : [hour, day];
  const slope = (high.at - low.at) / Math.log(high.ms / low.ms);
  return Math.min(1, Math.max(0, low.at + slope * Math.log(ms / low.ms)));
}

/** 「47 分」「2 时 14 分」「1 天 3 时」；不满 1 分钟写「不到 1 分」。只取最大的两个单位，零头不写。 */
export function waitLabel(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / MINUTE_MS);
  if (minutes < 1) return '不到 1 分';
  if (minutes < 60) return `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const rest = minutes % 60;
    return rest > 0 ? `${hours} 时 ${rest} 分` : `${hours} 时`;
  }
  const days = Math.floor(hours / 24);
  const rest = hours % 24;
  return rest > 0 ? `${days} 天 ${rest} 时` : `${days} 天`;
}

/** 「12 分前」「3 时前」「2 天前」：只取最大的一个单位；不满 1 分钟写「不到 1 分前」。 */
export function agoLabel(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / MINUTE_MS);
  if (minutes < 1) return '不到 1 分前';
  if (minutes < 60) return `${minutes} 分前`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} 时前` : `${Math.floor(hours / 24)} 天前`;
}
