import { byteLength } from '../canonicalModel';
import type { HistoryEntry, HistoryFile, HistoryPage } from '../historyModel';
import { canonical } from './canonicalFixtures';

export const historyContent = '# 历史原文\n\n<script>纯文本，不执行</script>\n';
export const historyEntry: HistoryEntry = {
  id: 'opaque-entry:一',
  revision: 'b'.repeat(64),
  bytes: byteLength(historyContent),
  authored_at_ms: null,
  author: null,
  archived_at_ms: 1789140000000,
  archived_by: {
    kind: 'human',
    issuer: 'https://fixture.invalid',
    subject: '虚构归档者',
  },
};
export const historyPage: HistoryPage = {
  vault_uuid: canonical.vault_uuid,
  path: canonical.path,
  entries: [historyEntry],
  next_cursor: null,
};
export const historyFile: HistoryFile = {
  vault_uuid: canonical.vault_uuid,
  path: canonical.path,
  revision: historyEntry.revision,
  content: historyContent,
};
