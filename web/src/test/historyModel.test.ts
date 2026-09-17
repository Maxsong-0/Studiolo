import { expect, it } from 'vitest';
import {
  historyFile as parseFile,
  historyPage as parsePage,
} from '../historyModel';
import { canonicalDocument, editingExplanation } from '../canonicalModel';
import { canonical } from './canonicalFixtures';
import { historyEntry, historyFile, historyPage } from './historyFixtures';

it('保留未知主体/时间和独立归档信息，重复 revision 的不同条目合法', () => {
  const value = {
    ...historyPage,
    entries: [historyEntry, { ...historyEntry, id: '再次提交', bytes: 999999 }],
  };
  expect(parsePage(value, canonical, null)).toEqual(value);
  expect(parseFile(historyFile, canonical, historyEntry)).toEqual(historyFile);
});
it.each([
  { ...historyPage, path: '另一页.md' },
  { ...historyPage, vault_uuid: '另一库' },
  { ...historyPage, entries: null },
  {
    ...historyPage,
    entries: Array.from({ length: 21 }, (_, index) => ({
      ...historyEntry,
      id: String(index),
    })),
  },
  { ...historyPage, entries: [historyEntry, historyEntry] },
  ...['', '\n', 'cursor\n', '中文', 'a'.repeat(1025), [], ['cursor']].map(
    (next_cursor) => ({ ...historyPage, next_cursor }),
  ),
])('拒绝跨作用域、过大/重复页或损坏游标 %j', (value) =>
  expect(() => parsePage(value, canonical, null)).toThrow(),
);
it.each([
  { id: '' },
  { id: 'a'.repeat(257) },
  { id: '条目\0' },
  { revision: ['b'.repeat(64)] },
  { revision: 'b'.repeat(64) + '\n' },
  { bytes: -1 },
  { bytes: 1.5 },
  { bytes: Number.MAX_SAFE_INTEGER + 1 },
  { authored_at_ms: '昨天' },
  { archived_at_ms: Number.NaN },
  { author: undefined },
  { author: [] },
  { author: { kind: ['obsidian'] } },
  { author: { kind: 'obsidian', subject: '不应存在' } },
  {
    author: { kind: 'human', issuer: 'http://fixture.invalid', subject: '人' },
  },
  {
    author: {
      kind: 'human',
      issuer: 'https://' + '中'.repeat(200),
      subject: '人',
    },
  },
  {
    author: {
      kind: 'human',
      issuer: 'https://fixture.invalid',
      subject: '中'.repeat(86),
    },
  },
  {
    archived_by: {
      kind: 'human',
      issuer: 'https://fixture.invalid',
      subject: '人\u0085',
    },
  },
])('历史元数据损坏不得作为恢复依据 %j', (change) =>
  expect(() =>
    parsePage(
      { ...historyPage, entries: [{ ...historyEntry, ...change }] },
      canonical,
      null,
    ),
  ).toThrow(),
);
it('游标不可循环；历史正文必须精确对应库/路径/版本与字节数', () => {
  expect(() =>
    parsePage({ ...historyPage, next_cursor: 'same' }, canonical, 'same'),
  ).toThrow();
  for (const value of [
    { ...historyFile, path: '另一页.md' },
    { ...historyFile, vault_uuid: '另一库' },
    { ...historyFile, revision: 'c'.repeat(64) },
    { ...historyFile, content: '\0' },
    { ...historyFile, content: 'a'.repeat(262145) },
    { ...historyFile, content: '少字节' },
  ])
    expect(() => parseFile(value, canonical, historyEntry)).toThrow();
});
it('恢复审计失效是独立闭集编辑锁，不把它当普通可写', () => {
  const document = canonicalDocument(
    {
      ...canonical,
      editing: {
        allowed: false,
        reason: 'audit_unavailable',
        max_bytes: 262144,
      },
    },
    canonical.path,
  );
  expect(editingExplanation(document)).toContain('恢复审计');
  expect(() =>
    canonicalDocument(
      { ...document, editing: { ...document.editing, allowed: true } },
      canonical.path,
    ),
  ).toThrow();
});
