import { ApiError, getJson } from './sessionHttp';
import {
  fileOperationCapability,
  fileOperationReceipt,
  tombstonePage,
} from './fileOperationValidation';
import type {
  FileOperationRecord,
  FileOperationRequest,
} from './fileOperationModel';

type Writer = (
  request: FileOperationRequest,
  signal: AbortSignal,
) => Promise<unknown>;
let writer: Writer | null = null;
/** 与正文写入相同，只接受 SessionBoundary 装配，不回退到直接 fetch。 */
export function installFileOperationWriter(next: Writer) {
  writer = next;
  return () => {
    if (writer === next) writer = null;
  };
}
export async function discoverFileOperations(signal: AbortSignal) {
  return fileOperationCapability(
    await getJson('/api/v1/file-operations', signal),
  );
}
export async function readTombstones(
  vault: string,
  cursor: string | null,
  signal: AbortSignal,
) {
  const query = new URLSearchParams({ vault_uuid: vault, limit: '32' });
  if (cursor !== null) query.set('cursor', cursor);
  return tombstonePage(
    await getJson(`/api/v1/tombstones?${query}`, signal),
    cursor,
  );
}
export async function mutateFile(
  request: FileOperationRequest,
  signal: AbortSignal,
  source?: FileOperationRecord['restoreSource'],
) {
  if (!writer) throw new ApiError('受控文件操作尚未连接，请重新登录。', 403);
  if (
    request.kind === 'restore_tombstone' &&
    (!source ||
      source.id !== request.tombstone_id ||
      source.path !== request.path)
  )
    throw new ApiError('请重新选择原路径的删除记录。', 400);
  return fileOperationReceipt(await writer(request, signal), request, source);
}
