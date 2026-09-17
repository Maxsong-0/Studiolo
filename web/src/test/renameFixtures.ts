import type { FileRenameRequest } from '../fileOperationModel';
import { canonical } from './canonicalFixtures';
export const renaming: FileRenameRequest = {
  kind: 'rename',
  vault_uuid: canonical.vault_uuid,
  from: canonical.path,
  to: '笔记/改名后的页面.md',
  expected_revision: canonical.revision,
  idempotency_key: 'rename-request-00001',
};
// 故意可变的未知 wire 对象，供负测构造畸形 presence；不冒充产品回执类型。
function effect(
  path: string,
  before: Record<string, unknown>,
  after: Record<string, unknown>,
) {
  return { kind: 'confirmed', path, before, after };
}
export function renameResponse(request = renaming) {
  const before = {
    kind: 'present',
    revision: request.expected_revision,
    sha256: 'c'.repeat(64),
    bytes: 42,
  };
  return {
    status: 'committed',
    receipt: {
      operation_id: 'd'.repeat(64),
      sequence: '7',
      outcome: 'committed',
      effects: [
        effect(
          request.to,
          { kind: 'missing' },
          { ...before, revision: 'b'.repeat(64) },
        ),
        effect(request.from, before, { kind: 'missing' }),
      ],
      tombstone_id: 'd'.repeat(64),
      evidence_id: null,
    },
  };
}
