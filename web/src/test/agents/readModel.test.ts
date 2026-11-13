import { expect, it } from 'vitest';
import { readAuditBody } from '../../../dev/fixtures/readAudit';
import { parseReadAudit } from '../../agents/readModel';
const now = Date.parse('2026-11-13T12:00:00Z');
it('封闭解码成功与匿名拒绝，游标不重叠且空页仍保留更早入口', () => {
  const page = parseReadAudit(readAuditBody({ outcome: 'read' }, now), 'read');
  const older = parseReadAudit(
    readAuditBody({ outcome: 'read', before: page.next_before }, now),
    'read',
  );
  expect(page.scanned).toBe(256);
  expect(Math.min(...page.records.map((r) => r.offset))).toBeGreaterThan(
    Math.max(...older.records.map((r) => r.offset)),
  );
  const denied = parseReadAudit(
    readAuditBody({ outcome: 'denied' }, now),
    'denied',
  );
  expect(denied.records.some((r) => r.agent_type === null)).toBe(true);
  expect(denied.records.every((r) => r.path === null)).toBe(true);
  const empty = parseReadAudit(
    readAuditBody({ outcome: 'read', path_prefix: '无匹配/' }, now),
    'read',
  );
  expect(empty.records).toHaveLength(0);
  expect(empty.next_before).not.toBeNull();
  expect(() => parseReadAudit(page, 'read', 2)).toThrow();
});
it.each(['extra', 'secret', 'mixed', 'order', 'cursor', 'size'])(
  '拒绝%s响应破坏封闭契约',
  (kind) => {
    const page = readAuditBody({ outcome: 'read' }, now)!;
    const bad: Record<string, unknown> = structuredClone(page);
    if (kind === 'extra') bad.token_sha256 = 'secret';
    if (kind === 'secret') {
      Object.assign(page.records[0], { session_digest: 'secret' });
      bad.records = page.records;
    }
    if (kind === 'mixed') {
      page.records[0].reason = 'denied';
      bad.records = page.records;
    }
    if (kind === 'order') bad.records = [page.records[0], page.records[0]];
    if (kind === 'cursor') bad.next_before = 0;
    if (kind === 'size') bad.scanned = 257;
    expect(() => parseReadAudit(bad, 'read')).toThrow();
  },
);
