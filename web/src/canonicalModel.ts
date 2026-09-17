import { validFreshness } from './freshness';
import type { Freshness } from './freshness';

export const EDIT_LIMIT = 262144;
export type EditingReason =
  | 'read_only'
  | 'protected'
  | 'stale'
  | 'too_large'
  | 'invalid_document'
  | 'audit_unavailable';
export interface CanonicalDocument {
  vault_uuid: string;
  path: string;
  content: string;
  revision: string;
  editing: {
    allowed: boolean;
    reason: EditingReason | null;
    max_bytes: number;
  };
  freshness: Freshness | null;
}
export interface Replacement {
  kind: 'replace';
  vault_uuid: string;
  path: string;
  expected_revision: string;
  content: string;
  idempotency_key: string;
}
export interface Restoration extends Omit<Replacement, 'kind' | 'content'> {
  kind: 'restore';
  source_revision: string;
}
/** 历史正文只供内存回执核对；恢复 HTTP 请求必须排除此 content 字段。 */
export type PendingMutation = Replacement | (Restoration & { content: string });
export interface SaveReceipt {
  path: string;
  previous_revision: string | null;
  new_revision: string;
}
export interface RevisionConflict {
  code: 'revision_conflict';
  vault_uuid: string;
  path: string;
  expected_revision: string;
  actual_revision: string | null;
}
export const byteLength = (text: string) =>
  new TextEncoder().encode(text).length;
const revision = (value: unknown): value is string =>
  typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const uuid = (value: unknown): value is string =>
  typeof value === 'string' &&
  /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(value);
const record = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const reasons = [
  'read_only',
  'protected',
  'stale',
  'too_large',
  'invalid_document',
  'audit_unavailable',
];
function invalid(): never {
  throw new Error('权威文档响应格式无效，请保留草稿并重新核对。');
}

export function canonicalDocument(
  value: unknown,
  path: string,
): CanonicalDocument {
  if (
    !record(value) ||
    !uuid(value.vault_uuid) ||
    value.path !== path ||
    !revision(value.revision) ||
    typeof value.content !== 'string' ||
    value.content.length > 1048576 ||
    byteLength(value.content) > 1048576 ||
    /[\0\uD800-\uDFFF]/u.test(value.content)
  )
    invalid();
  const editing = Object.hasOwn(value, 'editing')
    ? value.editing
    : { allowed: false, reason: 'read_only', max_bytes: EDIT_LIMIT };
  if (
    !record(editing) ||
    typeof editing.allowed !== 'boolean' ||
    editing.max_bytes !== EDIT_LIMIT ||
    (editing.allowed
      ? editing.reason !== null
      : typeof editing.reason !== 'string' || !reasons.includes(editing.reason))
  )
    invalid();
  const freshness = Object.hasOwn(value, 'freshness') ? value.freshness : null;
  if (!validFreshness(freshness)) invalid();
  return { ...value, editing, freshness } as unknown as CanonicalDocument;
}

export function saveReceipt(
  value: unknown,
  request: Replacement | Restoration,
): SaveReceipt {
  if (!record(value) || value.status !== 'committed' || !record(value.receipt))
    invalid();
  const receipt = value.receipt;
  if (
    receipt.path !== request.path ||
    !revision(receipt.new_revision) ||
    !(
      receipt.previous_revision === null ||
      receipt.previous_revision === request.expected_revision
    )
  )
    invalid();
  return receipt as unknown as SaveReceipt;
}

export function revisionConflict(
  value: unknown,
  request: Replacement | Restoration,
): RevisionConflict | null {
  if (
    !record(value) ||
    value.code !== 'revision_conflict' ||
    value.vault_uuid !== request.vault_uuid ||
    value.path !== request.path ||
    value.expected_revision !== request.expected_revision ||
    !(value.actual_revision === null || revision(value.actual_revision))
  )
    return null;
  return value as unknown as RevisionConflict;
}

export function canEdit(document: CanonicalDocument): boolean {
  return (
    document.editing.allowed &&
    document.freshness?.stale === false &&
    byteLength(document.content) <= EDIT_LIMIT
  );
}

export function editingExplanation(document: CanonicalDocument): string {
  if (byteLength(document.content) > EDIT_LIMIT)
    return '文档超过 256 KiB，仅提供只读内容。';
  const messages: Record<EditingReason, string> = {
    read_only: '当前服务未开放此文档的编辑。',
    protected: '此文档或生成区受到保护。',
    stale: '内容尚未追平，请重新核验后编辑。',
    too_large: '文档超过 256 KiB，仅提供只读内容。',
    invalid_document: '文档格式需要维护核对，暂不能编辑。',
    audit_unavailable:
      '恢复审计暂不可用，正文写入被阻断。请维护恢复后重新核验。',
  };
  return document.editing.reason
    ? messages[document.editing.reason]
    : '新鲜度证据尚未确认，请重新核验。';
}
