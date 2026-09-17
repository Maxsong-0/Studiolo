import type {
  FileOperationRequest,
  TombstoneEntry,
} from '../fileOperationModel';
import { renameResponse } from './renameFixtures';
import { canonical } from './canonicalFixtures';
export const deleting: Extract<FileOperationRequest, { kind: 'delete' }> = {
  kind: 'delete',
  vault_uuid: canonical.vault_uuid,
  path: canonical.path,
  expected_revision: canonical.revision,
  idempotency_key: 'delete-request-00001',
};
export const tombstone: TombstoneEntry = {
  id: 'd'.repeat(64),
  path: canonical.path,
  revision: canonical.revision,
  sha256: 'c'.repeat(64),
  bytes: 42,
  deleted_at_ms: 1000,
  retain_until_ms: 604_801_000,
  deleted_by: { kind: 'obsidian' },
};
export const restoring: Extract<
  FileOperationRequest,
  { kind: 'restore_tombstone' }
> = {
  kind: 'restore_tombstone',
  vault_uuid: canonical.vault_uuid,
  path: canonical.path,
  tombstone_id: tombstone.id,
  idempotency_key: 'restore-request-00001',
};
export function fileReceipt(request: FileOperationRequest = deleting) {
  if (request.kind === 'rename') return renameResponse(request);
  const present = {
    kind: 'present',
    revision: canonical.revision,
    sha256: tombstone.sha256,
    bytes: tombstone.bytes,
  };
  return {
    status: 'committed',
    receipt: {
      operation_id: 'd'.repeat(64),
      sequence: '7',
      outcome: 'committed',
      effects: [
        {
          kind: 'confirmed',
          path: request.path,
          before: request.kind === 'delete' ? present : { kind: 'missing' },
          after: request.kind === 'delete' ? { kind: 'missing' } : present,
        },
      ],
      tombstone_id: request.kind === 'delete' ? 'd'.repeat(64) : null,
      evidence_id: null,
    },
  };
}
