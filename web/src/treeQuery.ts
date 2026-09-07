import { TREE_PAGE_SIZE, type TreeEntry, type TreeResponse } from './treeTypes';

const DECIMAL_BYTES = /^(0|[1-9][0-9]{0,19})$/;
const MAX_BYTES = 18446744073709551615n;

function integer(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function name(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    !value.startsWith('.') &&
    value.normalize('NFC') === value &&
    !/[\\/]/.test(value) &&
    !/%(?:2f|5c|00|2e)/i.test(value) &&
    !Array.from(value).some((character) => {
      const point = character.codePointAt(0)!;
      // 与 Broker 的禁止范围一致，按码点比较，不把组合字符放进正则字符组。
      return (
        point < 32 ||
        (point >= 127 && point <= 159) ||
        (point >= 0x0300 && point <= 0x036f) ||
        (point >= 0x1ab0 && point <= 0x1aff) ||
        (point >= 0x1dc0 && point <= 0x1dff) ||
        (point >= 0x20d0 && point <= 0x20ff) ||
        (point >= 0xfe20 && point <= 0xfe2f)
      );
    })
  );
}

function entry(value: unknown, path: string): value is TreeEntry {
  if (!value || typeof value !== 'object') return false;
  const item = value as Record<string, unknown>;
  if (!name(item.name) || item.path !== `${path ? `${path}/` : ''}${item.name}`)
    return false;
  if (item.kind === 'directory')
    return item.size_bytes === null && integer(item.child_count);
  return (
    (item.kind === 'markdown' || item.kind === 'attachment') &&
    (item.kind === 'markdown') === /\.md$/i.test(item.name) &&
    item.child_count === null &&
    typeof item.size_bytes === 'string' &&
    DECIMAL_BYTES.test(item.size_bytes) &&
    BigInt(item.size_bytes) <= MAX_BYTES
  );
}

export function validateTree(value: unknown): TreeResponse {
  const invalid = () => new Error('文件树数据格式不完整，请重试。');
  if (!value || typeof value !== 'object') throw invalid();
  const response = value as Record<string, unknown>;
  if (
    !integer(response.generation) ||
    typeof response.path !== 'string' ||
    /^[a-z]:\//i.test(response.path) ||
    (response.path !== '' && !response.path.split('/').every(name)) ||
    !integer(response.offset) ||
    !integer(response.total) ||
    (response.offset !== 0 && response.offset >= response.total) ||
    !Array.isArray(response.entries) ||
    response.entries.length !==
      Math.min(TREE_PAGE_SIZE, response.total - response.offset)
  )
    throw invalid();
  const paths = new Set<string>();
  let filesStarted = false;
  for (const value of response.entries) {
    if (!entry(value, response.path) || paths.has(value.path.toLowerCase()))
      throw invalid();
    if (value.kind === 'directory' && filesStarted) throw invalid();
    if (value.kind !== 'directory') filesStarted = true;
    paths.add(value.path.toLowerCase());
  }
  const end = response.offset + response.entries.length;
  if (response.next_offset !== (end < response.total ? end : null))
    throw invalid();
  return response as unknown as TreeResponse;
}
