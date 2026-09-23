/**
 * 假数据的时间轴。所有时间都相对「此刻」算出来，租约、新鲜度、窗口才不会一过期就整页报错。
 *
 * 画布上的时间线是当天 08:00–20:00。这里把 08:00–20:00 这 12 小时整体挂在「这条时间带的终点」上：
 * 晚上 8 点以后启动，终点就是当天 20:00，事件正好落在 08:00–20:00；更早启动时终点取此刻，
 * 事件落在「截至此刻的 12 小时」里，保证没有任何事件晚于此刻。
 */
export const SECOND = 1_000;
export const MINUTE = 60 * SECOND;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;

/** 时间带终点（毫秒，整分钟）：min(此刻, 当天本地 20:00)。 */
export function dayEnd(now: number): number {
  const local = new Date(now);
  const eight = new Date(
    local.getFullYear(),
    local.getMonth(),
    local.getDate(),
    20,
  ).getTime();
  return Math.min(Math.floor(now / MINUTE) * MINUTE, eight);
}

/** 画布钟点（8.0 = 08:00，20.0 = 20:00）换算成毫秒时间戳。 */
export function clockAt(now: number, hour: number): number {
  return dayEnd(now) - Math.round((20 - hour) * HOUR);
}

/** 与后端 jiff 一致：整秒不带小数部分。 */
export function iso(ms: number): string {
  return new Date(ms).toISOString().replace('.000Z', 'Z');
}

/** UTC 日期（YYYY-MM-DD），项目日志按它归组。 */
export function utcDate(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}
