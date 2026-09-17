import { expect, it } from 'vitest';
import {
  fileOperationCapability,
  fileOperationReceipt,
  tombstonePage,
} from '../fileOperationValidation';
import {
  deleting,
  fileReceipt,
  restoring,
  tombstone,
} from './fileOperationFixtures';
import { canonical } from './canonicalFixtures';

it('能力发现只接受真实格式的Vault绑定和启用事实', () => {
  expect(
    fileOperationCapability({
      enabled: true,
      vault_uuid: canonical.vault_uuid,
    }),
  ).toEqual({
    enabled: true,
    vault_uuid: canonical.vault_uuid,
    rename_enabled: false,
  });
  for (const value of [
    null,
    [],
    { enabled: false, vault_uuid: canonical.vault_uuid },
    { enabled: true, vault_uuid: 'x' },
  ])
    expect(() => fileOperationCapability(value)).toThrow();
});
it('删除回执绑定路径和精确旧版本，Missing不能用空文件冒充', () => {
  expect(fileOperationReceipt(fileReceipt(), deleting).sequence).toBe('7');
  for (const mutate of [
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.effects[0].path = '别处.md';
    },
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.effects[0].before = {
        ...r.receipt.effects[0].before,
        revision: 'e'.repeat(64),
      };
    },
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.effects[0].after = {
        kind: 'present',
        revision: 'b'.repeat(64),
        sha256: 'c'.repeat(64),
        bytes: 0,
      };
    },
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.sequence = '18446744073709551616';
    },
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.sequence = '01';
    },
    (r: ReturnType<typeof fileReceipt>) => {
      r.receipt.tombstone_id = 'e'.repeat(64);
    },
  ]) {
    const value = fileReceipt();
    mutate(value);
    expect(() => fileOperationReceipt(value, deleting)).toThrow();
  }
});
it('恢复回执必须与选中墓碑的字节和版本相符', () => {
  expect(
    fileOperationReceipt(fileReceipt(restoring), restoring, tombstone)
      .tombstone_id,
  ).toBeNull();
  for (const source of [
    undefined,
    { ...tombstone, id: 'e'.repeat(64) },
    { ...tombstone, bytes: 0 },
    { ...tombstone, revision: 'e'.repeat(64) },
    { ...tombstone, sha256: 'e'.repeat(64) },
  ])
    expect(() =>
      fileOperationReceipt(fileReceipt(restoring), restoring, source),
    ).toThrow();
});
it('有界分页允许空页继续，但拒绝重复游标、重复墓碑和非法元数据', () => {
  const next = `d1:${'a'.repeat(64)}:32`;
  expect(
    tombstonePage({ entries: [], next_cursor: next }, null).next_cursor,
  ).toBe(next);
  expect(() =>
    tombstonePage({ entries: [], next_cursor: next }, next),
  ).toThrow();
  for (const entries of [
    [tombstone, tombstone],
    Array(33).fill(tombstone),
    [{ ...tombstone, path: '../secret' }],
    [{ ...tombstone, bytes: 268_435_457 }],
    [{ ...tombstone, retain_until_ms: 1001 }],
    [{ ...tombstone, deleted_by: null }],
  ])
    expect(() => tombstonePage({ entries, next_cursor: null }, null)).toThrow();
});
