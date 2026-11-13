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
