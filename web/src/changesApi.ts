import { ApiError, getJson } from './sessionHttp';
import {
  changesPage,
  seenReceipt,
  type ChangesPage,
  type SeenRequest,
} from './changesModel';
type Writer = (request: SeenRequest, signal: AbortSignal) => Promise<unknown>;
let writer: Writer | null = null;
export function installChangesWriter(next: Writer) {
  writer = next;
  return () => {
    if (writer === next) writer = null;
  };
}
export async function readChanges(
  project: string | null,
  before: string | null,
  signal: AbortSignal,
  previous: ChangesPage | null = null,
) {
  const query = new URLSearchParams({ limit: '50' });
  if (project) query.set('project', project);
  if (before) query.set('before', before);
  if (previous) query.set('vault_uuid', previous.vault_uuid);
  return changesPage(
    await getJson(`/api/v1/changes?${query}`, signal),
    project,
    before,
    previous,
  );
}
export async function markChangesSeen(
  page: ChangesPage,
  requestId: string,
  signal: AbortSignal,
) {
  if (!writer) throw new ApiError('需要有效应用会话才能标记已读。', 403);
  const request: SeenRequest = {
    vault_uuid: page.vault_uuid,
    project: page.project,
    snapshot: page.snapshot,
    request_id: requestId,
  };
  seenReceipt(await writer(request, signal), page);
}
