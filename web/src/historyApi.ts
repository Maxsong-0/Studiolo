import { getJson } from './sessionHttp';
import { historyFile, historyPage } from './historyModel';
import type { HistoryEntry, HistoryScope } from './historyModel';

export async function readHistoryPage(
  scope: HistoryScope,
  cursor: string | null,
  signal: AbortSignal,
) {
  const query = new URLSearchParams({
    vault_uuid: scope.vault_uuid,
    path: scope.path,
    limit: '20',
  });
  if (cursor !== null) query.set('cursor', cursor);
  return historyPage(
    await getJson(`/api/v1/history?${query}`, signal),
    scope,
    cursor,
  );
}
export async function readHistoryFile(
  scope: HistoryScope,
  entry: HistoryEntry,
  signal: AbortSignal,
) {
  const query = new URLSearchParams({
    vault_uuid: scope.vault_uuid,
    path: scope.path,
    revision: entry.revision,
  });
  return historyFile(
    await getJson(`/api/v1/history/file?${query}`, signal),
    scope,
    entry,
  );
}
