import type { SearchResponse } from './types';

export function queryError(query: string): string | null {
  if (
    Array.from(query).some((character) => {
      const point = character.codePointAt(0)!;
      return point < 32 || (point >= 127 && point <= 159);
    })
  )
    return '搜索词不能包含换行或控制字符。';
  const trimmed = query.trim();
  if (
    Array.from(trimmed).length > 256 ||
    new TextEncoder().encode(trimmed).byteLength > 1024
  ) {
    return '搜索词最多 256 个字符、1024 字节，请缩短后重试。';
  }
  return null;
}

export function validateSearch(value: unknown): SearchResponse {
  if (!value || typeof value !== 'object')
    throw new Error('搜索数据格式不完整，请重试。');
  const response = value as Record<string, unknown>;
  const valid =
    Number.isSafeInteger(response.generation) &&
    Number(response.generation) >= 0 &&
    typeof response.truncated === 'boolean' &&
    Array.isArray(response.hits) &&
    response.hits.length <= 100 &&
    response.hits.every((hit: unknown) => {
      if (!hit || typeof hit !== 'object') return false;
      const entry = hit as Record<string, unknown>;
      return (
        ['path', 'title', 'snippet'].every(
          (field) => typeof entry[field] === 'string',
        ) &&
        Number.isSafeInteger(entry.line) &&
        Number(entry.line) > 0 &&
        ['path', 'content'].includes(String(entry.kind))
      );
    });
  if (!valid) throw new Error('搜索数据格式不完整，请重试。');
  return response as unknown as SearchResponse;
}
