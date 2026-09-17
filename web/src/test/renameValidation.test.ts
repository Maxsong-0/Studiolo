import { expect, it } from 'vitest';
import {
  fileOperationCapability,
  fileOperationReceipt,
} from '../fileOperationValidation';
import { renameTargetError } from '../renameValidation';
import { renameSnapshotReady } from '../renameNavigation';
import { renaming, renameResponse } from './renameFixtures';
import { canonical } from './canonicalFixtures';
import { fixture } from './fixtures';

it('旧能力仅启用删除恢复；Rename 必须独立 boolean，坏 present/未知字段拒绝', () => {
  const base = { enabled: true, vault_uuid: canonical.vault_uuid };
  expect(fileOperationCapability(base).rename_enabled).toBe(false);
  expect(
    fileOperationCapability({ ...base, rename_enabled: false }).rename_enabled,
  ).toBe(false);
  expect(
    fileOperationCapability({ ...base, rename_enabled: true }).rename_enabled,
  ).toBe(true);
  for (const value of [null, [], ['true'], 1, 'true', undefined, {}])
    expect(() =>
      fileOperationCapability({ ...base, rename_enabled: value }),
    ).toThrow();
  expect(() =>
    fileOperationCapability({
      ...base,
      rename_enabled: true,
      overwrite: false,
    }),
  ).toThrow();
});
it('完整双效果按目标/源绑定，同 raw/bytes 但 revision 不同', () => {
  const result = fileOperationReceipt(renameResponse(), renaming);
  expect(result.effects.map((effect) => effect.path)).toEqual([
    renaming.to,
    renaming.from,
  ]);
  expect(result.tombstone_id).toBe(result.operation_id);
  expect(() =>
    fileOperationReceipt(renameResponse(), {
      ...renaming,
      to: '笔记/第三处.md',
    }),
  ).toThrow();
});
// wire 故意作为未知 JSON 修改，测试不会用类型断言假设畸形数据已经合法。
const malformed: [
  string,
  (value: ReturnType<typeof renameResponse>) => unknown,
][] = [
  [
    '倒序',
    (v) => {
      v.receipt.effects.reverse();
      return v;
    },
  ],
  [
    '缺第二项',
    (v) => {
      v.receipt.effects.pop();
      return v;
    },
  ],
  [
    '重复目标',
    (v) => {
      v.receipt.effects[1] = v.receipt.effects[0];
      return v;
    },
  ],
  [
    '额外效果',
    (v) => {
      v.receipt.effects.push(v.receipt.effects[1]);
      return v;
    },
  ],
  [
    '不符源',
    (v) => {
      v.receipt.effects[1].path = '别处.md';
      return v;
    },
  ],
  [
    '不符源版本',
    (v) => {
      v.receipt.effects[1].before.revision = 'e'.repeat(64);
      return v;
    },
  ],
  [
    '相同版本',
    (v) => {
      v.receipt.effects[0].after.revision = renaming.expected_revision;
      return v;
    },
  ],
  [
    '不符raw',
    (v) => {
      v.receipt.effects[0].after.sha256 = 'e'.repeat(64);
      return v;
    },
  ],
  [
    '不符bytes',
    (v) => {
      v.receipt.effects[0].after.bytes = 43;
      return v;
    },
  ],
  [
    '已有目标',
    (v) => {
      v.receipt.effects[0].before = v.receipt.effects[1].before;
      return v;
    },
  ],
  [
    '仍有源',
    (v) => {
      v.receipt.effects[1].after = v.receipt.effects[1].before;
      return v;
    },
  ],
  [
    '未知效果',
    (v) => {
      v.receipt.effects[0].kind = 'unknown';
      return v;
    },
  ],
  [
    '不符墓碑',
    (v) => {
      v.receipt.tombstone_id = 'e'.repeat(64);
      return v;
    },
  ],
  [
    'u64溢出',
    (v) => {
      v.receipt.sequence = '18446744073709551616';
      return v;
    },
  ],
  [
    '错误sequence类型',
    (v) => ({ ...v, receipt: { ...v.receipt, sequence: 7 } }),
  ],
  [
    '数组outcome',
    (v) => ({ ...v, receipt: { ...v.receipt, outcome: ['committed'] } }),
  ],
  [
    'nullpresence',
    (v) => ({
      ...v,
      receipt: {
        ...v.receipt,
        effects: [
          { ...v.receipt.effects[0], before: null },
          v.receipt.effects[1],
        ],
      },
    }),
  ],
  [
    '证据未知',
    (v) => ({ ...v, receipt: { ...v.receipt, evidence_id: 'e'.repeat(64) } }),
  ],
  ['未知顶层', (v) => ({ ...v, verified: true })],
  ['未知回执字段', (v) => ({ ...v, receipt: { ...v.receipt, synced: true } })],
  [
    '未知effect字段',
    (v) => ({
      ...v,
      receipt: {
        ...v.receipt,
        effects: [
          { ...v.receipt.effects[0], confirmed: true },
          v.receipt.effects[1],
        ],
      },
    }),
  ],
];
it.each(malformed)('拒绝损坏 Rename 回执：%s', (_name, corrupt) => {
  expect(() =>
    fileOperationReceipt(corrupt(renameResponse()), renaming),
  ).toThrow();
});
it('目标输入原样核对，不自动trim、规范化或猜范围', () => {
  for (const to of [
    '',
    '/abs.md',
    '../x.md',
    '笔记//x.md',
    '笔记/./x.md',
    '笔记/x.MD',
    '笔记/x.pdf',
    '笔记/草稿.md',
    '笔记/草稿.md/child.md',
    '笔记/.hidden.md',
    'C:relative.md',
    '笔记/evil\u202ename.md',
    '笔记/evil\u0000.md',
    `笔记/${'文'.repeat(1400)}.md`,
    '笔记/e\u0301.md',
  ])
    expect(renameTargetError(renaming.from, to)).not.toBeNull();
  expect(renameTargetError(renaming.from, '笔记/v1.2.md')).toBeNull();
  expect(
    renameTargetError(renaming.from, '笔记/<img onerror=1>.md'),
  ).toBeNull();
});
it('快照只有真实fresh与双u64水位追平才可导航，不借generation或Number近似', () => {
  const freshness = {
    ...canonical.freshness!,
    indexed_through: '9007199254740992',
    committed_through: '9007199254740992',
  };
  const current = { ...fixture, freshness };
  expect(renameSnapshotReady(current, '9007199254740993')).toBe(false);
  expect(renameSnapshotReady(current, '9007199254740992')).toBe(true);
  for (const value of [
    null,
    { ...fixture },
    { ...current, freshness: null },
    { ...current, freshness: { ...freshness, stale: true, reason: 'changed' } },
    { ...current, freshness: { ...freshness, indexed_through: null } },
    {
      ...current,
      freshness: { ...freshness, committed_through: '9007199254740991' },
    },
  ])
    expect(renameSnapshotReady(value, '7')).toBe(false);
  for (const sequence of ['0', '01', '-1', '18446744073709551616'])
    expect(renameSnapshotReady(current, sequence)).toBe(false);
});

it('空文本也须是两份Present内容证明，不能与Missing混用', () => {
  const value = renameResponse();
  value.receipt.effects[0].after.bytes = 0;
  value.receipt.effects[1].before.bytes = 0;
  expect(fileOperationReceipt(value, renaming).effects).toHaveLength(2);
});
