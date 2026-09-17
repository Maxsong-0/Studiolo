import type {
  CanonicalDocument,
  Replacement,
  Restoration,
} from '../canonicalModel';
export const ownerScope = 'f'.repeat(64);
export const canonical: CanonicalDocument = {
  vault_uuid: '12345678-1234-1234-1234-123456789abc',
  path: '笔记/草稿.md',
  content: '---\n未知字段: 保留\n---\n# 基线\n',
  revision: 'a'.repeat(64),
  editing: { allowed: true, reason: null, max_bytes: 262144 },
  freshness: {
    stale: false,
    indexed_through: '2',
    committed_through: '2',
    unresolved: false,
    watcher_healthy: true,
    reason: null,
  },
};
export function committed(
  request: Replacement | Restoration,
  revision = 'b'.repeat(64),
) {
  return {
    status: 'committed',
    receipt: {
      path: request.path,
      previous_revision: request.expected_revision,
      new_revision: revision,
    },
  };
}
export const replacement: Replacement = {
  kind: 'replace',
  vault_uuid: canonical.vault_uuid,
  path: canonical.path,
  expected_revision: canonical.revision,
  content: '更新正文',
  idempotency_key: 'fictional-key-0001',
};
