/**
 * 「多久以前」的说法（T-196；纯函数）。
 * 不到 5 秒说「刚刚」；一分钟内按 5 秒一跳，免得数字一秒一变抢眼；之后按分钟、小时、天。
 */
const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const STEP = 5 * SECOND;

/** 本机时钟往回拨时会算出负数：按 0 算，不写「-3 秒前」。 */
const clamp = (elapsedMs: number) => Math.max(0, elapsedMs);

export function agoLabel(elapsedMs: number): string {
  const ms = clamp(elapsedMs);
  if (ms < STEP) return '刚刚';
  if (ms < MINUTE) return `${Math.floor(ms / STEP) * 5} 秒前`;
  if (ms < HOUR) return `${Math.floor(ms / MINUTE)} 分钟前`;
  if (ms < DAY) return `${Math.floor(ms / HOUR)} 小时前`;
  return `${Math.floor(ms / DAY)} 天前`;
}

/** 这句说法还能用多久（毫秒）：到点再算一次，不用每秒重画。 */
export function agoValidFor(elapsedMs: number): number {
  const ms = clamp(elapsedMs);
  const step = ms < MINUTE ? STEP : ms < HOUR ? MINUTE : ms < DAY ? HOUR : DAY;
  return step - (ms % step);
}
