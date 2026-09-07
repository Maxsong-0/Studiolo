import { expect, it } from 'vitest';
import { queryError, validateSearch } from '../searchQuery';

it('按Unicode字符而不是UTF-16长度限制查询，拒绝控制字符', () => {
  expect(queryError('😀'.repeat(256))).toBeNull();
  expect(queryError('😀'.repeat(257))).toContain('256');
  expect(queryError(' 正常查询 ')).toBeNull();
  expect(queryError('换\n行')).toContain('控制字符');
  expect(queryError('\t查询')).toContain('控制字符');
});

it('拒绝无效搜索结果行号、枚举和超过100条的响应', () => {
  const hit = { path: 'a.md', title: 'a', line: 1, snippet: '', kind: 'path' };
  const response = { generation: 1, hits: [hit], truncated: false };
  expect(validateSearch(response)).toEqual(response);
  expect(() =>
    validateSearch({ ...response, hits: [{ ...hit, line: 0 }] }),
  ).toThrow();
  expect(() =>
    validateSearch({ ...response, hits: [{ ...hit, kind: 'html' }] }),
  ).toThrow();
  expect(() =>
    validateSearch({
      ...response,
      hits: Array.from({ length: 101 }, () => hit),
    }),
  ).toThrow();
});
