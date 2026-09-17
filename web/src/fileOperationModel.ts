import type { HistoryActor } from './historyModel';

interface FileRequest {
  vault_uuid: string;
  idempotency_key: string;
}
export type FileOperationRequest = FileRequest &
  (
    | { kind: 'delete'; path: string; expected_revision: string }
    | { kind: 'restore_tombstone'; path: string; tombstone_id: string }
    | { kind: 'rename'; from: string; to: string; expected_revision: string }
  );
export type FileRenameRequest = Extract<
  FileOperationRequest,
  { kind: 'rename' }
>;
export interface FileOperationCapability {
  vault_uuid: string;
  enabled: true;
  rename_enabled: boolean;
}
export const fileOperationSource = (request: FileOperationRequest) =>
  request.kind === 'rename' ? request.from : request.path;
export type FilePresence =
  | { kind: 'missing' }
  | { kind: 'present'; revision: string; sha256: string; bytes: number };
export interface ConfirmedFileEffect {
  kind: 'confirmed';
  path: string;
  before: FilePresence;
  after: FilePresence;
}
export interface FileOperationReceipt {
  operation_id: string;
  sequence: string;
  outcome: 'committed';
  effects: [ConfirmedFileEffect] | [ConfirmedFileEffect, ConfirmedFileEffect];
  tombstone_id: string | null;
  evidence_id: null;
}
export interface TombstoneEntry {
  id: string;
  path: string;
  revision: string;
  sha256: string;
  bytes: number;
  deleted_at_ms: number | null;
  deleted_by: HistoryActor;
  retain_until_ms: number | null;
}
export interface TombstonePage {
  entries: TombstoneEntry[];
  next_cursor: string | null;
}
export interface FileOperationRecord {
  request: FileOperationRequest;
  restoreSource?: Pick<
    TombstoneEntry,
    'id' | 'path' | 'revision' | 'sha256' | 'bytes'
  >;
  phase: 'sending' | 'uncertain' | 'committed' | 'rejected';
  message: string;
  receipt?: FileOperationReceipt;
}

export const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);
export const isHex = (value: unknown): value is string =>
  typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
export const isUuid = (value: unknown): value is string =>
  typeof value === 'string' &&
  /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(value);
export function invalidFileOperation(): never {
  throw new Error('文件操作响应不完整，不能据此确认结果；请保留原请求。');
}
