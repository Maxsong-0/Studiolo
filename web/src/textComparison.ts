/** 共同首尾以外为变更区；线性比较，不声称给出最小编辑脚本。 */
export function changedLines(base: string, current: string) {
  const left = base.split('\n');
  const right = current.split('\n');
  if (left.length > 2000 || right.length > 2000) return null;
  let prefix = 0;
  while (
    prefix < left.length &&
    prefix < right.length &&
    left[prefix] === right[prefix]
  )
    prefix += 1;
  let suffix = 0;
  while (
    suffix < left.length - prefix &&
    suffix < right.length - prefix &&
    left[left.length - 1 - suffix] === right[right.length - 1 - suffix]
  )
    suffix += 1;
  return right.map((text, index) => ({
    text,
    changed: index >= prefix && index < right.length - suffix,
  }));
}
