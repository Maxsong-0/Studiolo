import { byteLength, EDIT_LIMIT } from './canonicalModel';

export interface HistoryScope {
  vault_uuid: string;
  path: string;
}
export type HistoryActor =
  { kind: 'human'; issuer: string; subject: string } | { kind: 'obsidian' };
export interface HistoryEntry {
  id: string;
  revision: string;
  bytes: number;
  authored_at_ms: number | null;
  author: HistoryActor | null;
  archived_at_ms: number | null;
  archived_by: HistoryActor | null;
}
export interface HistoryPage extends HistoryScope {
  entries: HistoryEntry[];
  next_cursor: string | null;
}
export interface HistoryFile extends HistoryScope {
  revision: string;
  content: string;
}
const record = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown): value is string =>
  typeof value === 'string' && value.length > 0;
const hex = (value: unknown): value is string =>
  typeof value === 'string' &&
  value.length === 64 &&
  /^[0-9a-f]{64}$/.test(value);
const integer = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value);
const time = (value: unknown) => value === null || integer(value);
const actor = (value: unknown) =>
  value === null ||
  (record(value) &&
    ((value.kind === 'obsidian' && Object.keys(value).length === 1) ||
      (value.kind === 'human' &&
        Object.keys(value).length === 3 &&
        text(value.issuer) &&
        value.issuer.startsWith('https://') &&
        byteLength(value.issuer) <= 512 &&
        !/\p{Cc}/u.test(value.issuer) &&
        text(value.subject) &&
        byteLength(value.subject) <= 256 &&
        !/\p{Cc}/u.test(value.subject))));
const cursor = (value: unknown) =>
  value === null ||
  (text(value) && value.length <= 1024 && !/[^\x20-\x7e]/.test(value));
function invalid(): never {
  throw new Error('页面历史响应格式无效，请重新核对，不能据此恢复。');
}
function scope(
  value: unknown,
  expected: HistoryScope,
): value is Record<string, unknown> {
  return (
    record(value) &&
    value.vault_uuid === expected.vault_uuid &&
    value.path === expected.path
  );
}
export function historyPage(
  value: unknown,
  expected: HistoryScope,
  requestedCursor: string | null,
): HistoryPage {
  if (
    !scope(value, expected) ||
    !Array.isArray(value.entries) ||
    value.entries.length > 20 ||
    !cursor(value.next_cursor) ||
    (requestedCursor !== null && value.next_cursor === requestedCursor)
  )
    invalid();
  const ids = new Set<string>();
  for (const entry of value.entries) {
    if (
      !record(entry) ||
      !text(entry.id) ||
      byteLength(entry.id) > 256 ||
      /\p{Cc}/u.test(entry.id) ||
      ids.has(entry.id) ||
      !hex(entry.revision) ||
      !integer(entry.bytes) ||
      entry.bytes < 0 ||
      !time(entry.authored_at_ms) ||
      !time(entry.archived_at_ms) ||
      !actor(entry.author) ||
      !actor(entry.archived_by)
    )
      invalid();
    ids.add(entry.id);
  }
  return value as unknown as HistoryPage;
}
export function historyFile(
  value: unknown,
  expected: HistoryScope,
  entry: Pick<HistoryEntry, 'revision' | 'bytes'>,
): HistoryFile {
  if (
    !scope(value, expected) ||
    value.revision !== entry.revision ||
    !hex(value.revision) ||
    typeof value.content !== 'string' ||
    value.content.length > EDIT_LIMIT ||
    byteLength(value.content) > EDIT_LIMIT ||
    byteLength(value.content) !== entry.bytes ||
    /[\0\uD800-\uDFFF]/u.test(value.content)
  )
    invalid();
  return value as unknown as HistoryFile;
}
