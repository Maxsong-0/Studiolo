/** 二进制字节单位，64位整数全程不转浮点数。 */
export function bytes(value: string | null): string {
  if (value === null) return '未知';
  const size = BigInt(value);
  for (const [unit, divisor] of [
    ['GiB', 1073741824n],
    ['MiB', 1048576n],
    ['KiB', 1024n],
  ] as const) {
    if (size >= divisor)
      return `${size / divisor}.${((size * 10n) / divisor) % 10n} ${unit}`;
  }
  return `${size} B`;
}
/** 健康明细里的时刻；后端给不出就写「未知」，不编一个。 */
export function localTime(value: string | null): string {
  return value === null
    ? '未知'
    : new Date(value).toLocaleString('zh-CN', { hour12: false });
}
/** 秒数的中文时长，和后端正文同一写法：一天以上精确到小时，一天以内精确到分钟。 */
export function duration(seconds: string): string {
  const minutes = BigInt(seconds) / 60n;
  const days = minutes / 1440n,
    hours = (minutes / 60n) % 24n,
    rest = minutes % 60n;
  if (days > 0n) return hours > 0n ? `${days} 天 ${hours} 小时` : `${days} 天`;
  if (hours > 0n)
    return rest > 0n ? `${hours} 小时 ${rest} 分钟` : `${hours} 小时`;
  return rest > 0n ? `${rest} 分钟` : '不到 1 分钟';
}
