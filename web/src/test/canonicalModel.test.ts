import { expect, it } from 'vitest';
import {
  canonicalDocument,
  canEdit,
  revisionConflict,
  saveReceipt,
} from '../canonicalModel';
import { canonical, committed, replacement } from './canonicalFixtures';

it('旧 canonical 响应可读，但缺编辑范围和新鲜度不能启用写入', () => {
  const { vault_uuid, path, content, revision } = canonical;
  expect(
    canEdit(canonicalDocument({ vault_uuid, path, content, revision }, path)),
  ).toBe(false);
  expect(canEdit(canonicalDocument(canonical, path))).toBe(true);
});
it.each([
  { ...canonical, revision: ['a'.repeat(64)] },
  { ...canonical, revision: 'a'.repeat(63) },
  { ...canonical, vault_uuid: [] },
  { ...canonical, path: '另一页.md' },
  { ...canonical, editing: null },
  { ...canonical, editing: [] },
  {
    ...canonical,
    editing: { allowed: true, reason: 'protected', max_bytes: 262144 },
  },
  {
    ...canonical,
    editing: { allowed: false, reason: ['stale'], max_bytes: 262144 },
  },
  {
    ...canonical,
    editing: { allowed: false, reason: null, max_bytes: 262144 },
  },
  { ...canonical, freshness: [] },
  { ...canonical, content: 'a'.repeat(1048577) },
  { ...canonical, content: '\0' },
])('拒绝损坏的权威文档 %j', (value) =>
  expect(() => canonicalDocument(value, canonical.path)).toThrow(),
);
it('回执必须绑定原请求路径、前版本与规范新版本', () => {
  expect(saveReceipt(committed(replacement), replacement).new_revision).toBe(
    'b'.repeat(64),
  );
  for (const receipt of [
    {
      path: '另一页.md',
      previous_revision: canonical.revision,
      new_revision: 'b'.repeat(64),
    },
    {
      path: canonical.path,
      previous_revision: 'c'.repeat(64),
      new_revision: 'b'.repeat(64),
    },
    {
      path: canonical.path,
      previous_revision: canonical.revision,
      new_revision: ['b'.repeat(64)],
    },
  ])
    expect(() =>
      saveReceipt({ status: 'committed', receipt }, replacement),
    ).toThrow();
});
it('409 证据绑定本请求，不接受跨页或跨 Vault 冲突', () => {
  const value = {
    ...replacement,
    code: 'revision_conflict',
    actual_revision: null,
  };
  expect(revisionConflict(value, replacement)).not.toBeNull();
  expect(
    revisionConflict({ ...value, vault_uuid: '另一库' }, replacement),
  ).toBeNull();
  expect(
    revisionConflict({ ...value, path: '另一页.md' }, replacement),
  ).toBeNull();
});
