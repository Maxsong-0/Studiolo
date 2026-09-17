import { byteLength } from './canonicalModel';
import {
  invalidFileOperation as invalid,
  isHex,
  isRecord,
  isUuid,
} from './fileOperationModel';
import type {
  FileOperationCapability,
  FileOperationReceipt,
  FileOperationRecord,
  FileOperationRequest,
  TombstonePage,
} from './fileOperationModel';

import { renameReceipt } from './renameValidation';
import {
  validFileBytes,
  validFilePath,
  validFilePresence,
} from './fileOperationPrimitives';

const time = (value: unknown) =>
  value === null ||
  (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
const actor = (value: unknown) =>
  isRecord(value) &&
  ((value.kind === 'obsidian' && Object.keys(value).length === 1) ||
    (value.kind === 'human' &&
      Object.keys(value).length === 3 &&
      typeof value.issuer === 'string' &&
      value.issuer.startsWith('https://') &&
      byteLength(value.issuer) <= 512 &&
      !/\p{Cc}/u.test(value.issuer) &&
      typeof value.subject === 'string' &&
      value.subject.length > 0 &&
      byteLength(value.subject) <= 256 &&
      !/\p{Cc}/u.test(value.subject)));
const cursor = (value: unknown): value is string | null =>
  value === null ||
  (typeof value === 'string' &&
    value.length <= 100 &&
    /^d1:[0-9a-f]{64}:(0|[1-9][0-9]*)$/.test(value));

export function fileOperationCapability(
  value: unknown,
): FileOperationCapability {
  if (
    !isRecord(value) ||
    value.enabled !== true ||
    !isUuid(value.vault_uuid) ||
    Object.keys(value).some(
      (key) => !['vault_uuid', 'enabled', 'rename_enabled'].includes(key),
    ) ||
    (Object.hasOwn(value, 'rename_enabled') &&
      typeof value.rename_enabled !== 'boolean')
  )
    invalid();
  return {
    vault_uuid: value.vault_uuid,
    enabled: true,
    rename_enabled: value.rename_enabled === true,
  };
}
export function tombstonePage(
  value: unknown,
  requestedCursor: string | null,
): TombstonePage {
  if (
    !isRecord(value) ||
    !Array.isArray(value.entries) ||
    value.entries.length > 32 ||
    !cursor(value.next_cursor) ||
    (requestedCursor !== null && requestedCursor === value.next_cursor)
  )
    invalid();
  const ids = new Set<string>();
  for (const entry of value.entries) {
    if (
      !isRecord(entry) ||
      !isHex(entry.id) ||
      ids.has(entry.id) ||
      !validFilePath(entry.path) ||
      !isHex(entry.revision) ||
      !isHex(entry.sha256) ||
      !validFileBytes(entry.bytes) ||
      !time(entry.deleted_at_ms) ||
      !time(entry.retain_until_ms) ||
      !actor(entry.deleted_by) ||
      (typeof entry.deleted_at_ms === 'number' &&
        typeof entry.retain_until_ms === 'number' &&
        entry.retain_until_ms < entry.deleted_at_ms + 604_800_000)
    )
      invalid();
    ids.add(entry.id);
  }
  return value as unknown as TombstonePage;
}
export function fileOperationReceipt(
  value: unknown,
  request: FileOperationRequest,
  source?: FileOperationRecord['restoreSource'],
): FileOperationReceipt {
  if (request.kind === 'rename') return renameReceipt(value, request);
  if (
    !isRecord(value) ||
    value.status !== 'committed' ||
    !isRecord(value.receipt)
  )
    invalid();
  const r = value.receipt;
  if (
    !isHex(r.operation_id) ||
    typeof r.sequence !== 'string' ||
    !/^[1-9][0-9]{0,19}$/.test(r.sequence) ||
    (r.sequence.length === 20 && r.sequence > '18446744073709551615') ||
    r.outcome !== 'committed' ||
    r.evidence_id !== null ||
    !Array.isArray(r.effects) ||
    r.effects.length !== 1
  )
    invalid();
  const effect: unknown = r.effects[0];
  if (
    !isRecord(effect) ||
    effect.kind !== 'confirmed' ||
    effect.path !== request.path ||
    !validFilePresence(effect.before) ||
    !validFilePresence(effect.after)
  )
    invalid();
  const before = effect.before as Record<string, unknown>;
  const after = effect.after as Record<string, unknown>;
  if (
    request.kind === 'delete'
      ? before.kind !== 'present' ||
        before.revision !== request.expected_revision ||
        after.kind !== 'missing' ||
        r.tombstone_id !== r.operation_id
      : !source ||
        source.id !== request.tombstone_id ||
        source.path !== request.path ||
        before.kind !== 'missing' ||
        after.kind !== 'present' ||
        r.tombstone_id !== null ||
        after.revision !== source.revision ||
        after.sha256 !== source.sha256 ||
        after.bytes !== source.bytes
  )
    invalid();
  return r as unknown as FileOperationReceipt;
}
