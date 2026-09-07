import { expect, it, vi } from 'vitest';
import { getTree } from '../api';
import { validateTree } from '../treeQuery';
import { jsonResponse } from './fixtures';
import { treeEntry, treePage } from './treeFixtures';

it('接受空根、空目录和大于安全整数的精确附件字节数', () => {
  expect(validateTree(treePage('', [])).entries).toEqual([]);
  expect(validateTree(treePage('资料', [])).total).toBe(0);
  const value = treePage('资料', [treeEntry('资料/原始.bin')]);
  expect(validateTree(value).entries[0].size_bytes).toBe('9007199254740993');
});

it('拒绝非直接子项、重复项、路径穿越、非法名称及目录文件混合字段', () => {
  const attachment = treeEntry('资料/文件.pdf');
  const invalidEntries = [
    { ...attachment, path: '其他/文件.pdf' },
    { ...attachment, path: '资料/子目录/文件.pdf' },
    { ...attachment, name: '..', path: '资料/..' },
    { ...attachment, name: 'a\\b', path: '资料/a\\b' },
    { ...attachment, name: 'a\nb', path: '资料/a\nb' },
    { ...attachment, name: '.隐藏', path: '资料/.隐藏' },
    { ...attachment, name: 'e\u0301.pdf', path: '资料/e\u0301.pdf' },
    { ...attachment, name: '%2f.pdf', path: '资料/%2f.pdf' },
    { ...attachment, kind: 'image' },
    { ...attachment, kind: 'markdown' },
    treeEntry('资料/笔记.MD', 'attachment'),
    { ...attachment, size_bytes: 12 },
    { ...attachment, size_bytes: '01' },
    { ...attachment, size_bytes: '-1' },
    { ...attachment, size_bytes: '18446744073709551616' },
    { ...attachment, child_count: 0 },
    { ...treeEntry('资料/子目录', 'directory'), size_bytes: '0' },
    { ...treeEntry('资料/子目录', 'directory'), child_count: -1 },
  ];
  for (const entry of invalidEntries)
    expect(() =>
      validateTree(treePage('资料', [entry as typeof attachment])),
    ).toThrow();
  expect(() =>
    validateTree(treePage('资料', [attachment, attachment])),
  ).toThrow();
  expect(() =>
    validateTree(
      treePage('资料', [attachment, treeEntry('资料/子目录', 'directory')]),
    ),
  ).toThrow();
  expect(() => validateTree(treePage('../资料', []))).toThrow();
  expect(() => validateTree(treePage('C:/资料', []))).toThrow();
  for (const point of [
    0x0300, 0x036f, 0x1ab0, 0x1aff, 0x1dc0, 0x1dff, 0x20d0, 0x20ff, 0xfe20,
    0xfe2f,
  ]) {
    const entry = treeEntry(`资料/附件${String.fromCodePoint(point)}.bin`);
    expect(() => validateTree(treePage('资料', [entry]))).toThrow();
  }
});

it('严格验证分页长度、页界、总数和下一页，不接受伪造或无穷分页', () => {
  const entries = Array.from({ length: 200 }, (_, index) =>
    treeEntry(`项${index}`),
  );
  const first = treePage('', entries, { total: 201, next_offset: 200 });
  expect(validateTree(first).next_offset).toBe(200);
  const final = treePage('', [treeEntry('末项')], { offset: 200, total: 201 });
  expect(validateTree(final).offset).toBe(200);
  for (const value of [
    { ...first, next_offset: 0 },
    { ...first, next_offset: null },
    { ...first, total: 199 },
    { ...first, offset: 0.5 },
    { ...first, entries: entries.slice(1) },
    { ...first, entries: [...entries, treeEntry('额外项')] },
    { ...first, generation: -1 },
    { ...first, total: Number.MAX_SAFE_INTEGER + 1 },
    { ...final, offset: 400 },
    treePage('', [], { total: 200, offset: 200 }),
  ])
    expect(() => validateTree(value)).toThrow();
  expect(validateTree({ ...first, offset: 1, next_offset: null }).offset).toBe(
    1,
  );
});

it('接口编码完整目录路径并核对请求位置与代次', async () => {
  const path = '资料/中文 #问号?';
  const fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal('fetch', fetchMock);
  const signal = new AbortController().signal;
  fetchMock.mockResolvedValueOnce(jsonResponse(treePage(path, [])));
  await getTree(path, 0, 7, signal);
  const url = new URL(String(fetchMock.mock.calls[0][0]), 'http://localhost');
  expect(url.pathname).toBe('/api/tree');
  expect(url.searchParams.get('path')).toBe(path);
  expect(url.searchParams.get('offset')).toBe('0');
  expect(url.searchParams.get('generation')).toBe('7');
  expect(fetchMock.mock.calls[0][1]?.signal).toBe(signal);
  fetchMock.mockResolvedValueOnce(jsonResponse(treePage('别处', [])));
  await expect(getTree(path, 0, 7, signal)).rejects.toThrow('位置与请求不一致');
  fetchMock.mockResolvedValueOnce(
    jsonResponse(treePage(path, [], { generation: 6 })),
  );
  await expect(getTree(path, 0, 7, signal)).rejects.toMatchObject({
    status: 409,
  });
  fetchMock.mockResolvedValueOnce(
    jsonResponse(
      treePage(path, [treeEntry(`${path}/末项.pdf`)], {
        offset: 200,
        total: 201,
      }),
    ),
  );
  await expect(getTree(path, 0, 7, signal)).rejects.toThrow('位置与请求不一致');
});
