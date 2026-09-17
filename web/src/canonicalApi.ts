import { ApiError, getJson } from './sessionHttp';
import { canonicalDocument, saveReceipt } from './canonicalModel';
import type {
  PendingMutation,
  Replacement,
  Restoration,
} from './canonicalModel';

type Writer = (request: Replacement, signal: AbortSignal) => Promise<unknown>;
let writer: Writer | null = null;
type Restorer = (request: Restoration, signal: AbortSignal) => Promise<unknown>;
let restorer: Restorer | null = null;

/** 仅 SessionBoundary 可装配写者；没有直接 fetch 或旧只读模式的写入回退。 */
export function installCanonicalWriter(next: Writer) {
  writer = next;
  return () => {
    if (writer === next) writer = null;
  };
}
export function installCanonicalRestorer(next: Restorer) {
  restorer = next;
  return () => {
    if (restorer === next) restorer = null;
  };
}
export async function readCanonical(path: string, signal: AbortSignal) {
  return canonicalDocument(
    await getJson(`/api/v1/file?${new URLSearchParams({ path })}`, signal),
    path,
  );
}
/** 保存后的索引可能仍在追平；仅有界重读，不重放 POST。 */
export async function readAfterCommit(path: string, signal: AbortSignal) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      const document = await readCanonical(path, signal);
      if (
        attempt >= 2 ||
        (document.freshness?.stale !== true &&
          document.editing.reason !== 'stale')
      )
        return document;
    } catch (error) {
      if (
        signal.aborted ||
        attempt >= 2 ||
        !(error instanceof ApiError) ||
        ![429, 503].includes(error.status)
      )
        throw error;
    }
    await new Promise<void>((resolve, reject) => {
      const cancel = () => {
        clearTimeout(timer);
        signal.removeEventListener('abort', cancel);
        reject(new DOMException('读取已取消。', 'AbortError'));
      };
      const timer = setTimeout(
        () => {
          signal.removeEventListener('abort', cancel);
          resolve();
        },
        attempt ? 450 : 150,
      );
      signal.addEventListener('abort', cancel, { once: true });
      if (signal.aborted) cancel();
    });
  }
}
export async function replaceCanonical(
  request: PendingMutation,
  signal: AbortSignal,
) {
  if (request.kind === 'restore') {
    if (!restorer)
      throw new ApiError('尚未装配受控历史恢复，请重新登录。', 403);
    const body: Restoration = {
      kind: 'restore',
      vault_uuid: request.vault_uuid,
      path: request.path,
      expected_revision: request.expected_revision,
      source_revision: request.source_revision,
      idempotency_key: request.idempotency_key,
    };
    return saveReceipt(await restorer(body, signal), body);
  }
  if (!writer) throw new ApiError('尚未装配受控会话写入，请重新登录。', 403);
  return saveReceipt(await writer(request, signal), request);
}
