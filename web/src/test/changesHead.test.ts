import { expect, it } from 'vitest';
import { changesHead, sameChanges } from '../changesHead';

const VAULT = '00000000-0000-4000-8000-000000000001';
const through = {
  sequence: '21',
  epoch: 'd11d3d3a-2943-4cae-bf20-000000000001',
  observation: '2',
  generation: '243',
};

it('水位只收库身份和四项水位，多一项、少一项、类型不对都拒绝', () => {
  expect(changesHead({ vault_uuid: VAULT, through }, VAULT)).toEqual({
    vault_uuid: VAULT,
    through,
  });
  expect(changesHead({ vault_uuid: VAULT, through }, null).vault_uuid).toBe(
    VAULT,
  );
  for (const bad of [
    null,
    [],
    {},
    { vault_uuid: VAULT },
    { vault_uuid: VAULT, through, entries: [] },
    { vault_uuid: 'x', through },
    { vault_uuid: VAULT, through: { ...through, sequence: 21 } },
    { vault_uuid: VAULT, through: { ...through, extra: '1' } },
  ])
    expect(() => changesHead(bad, null)).toThrow('变更水位响应格式无效');
  expect(() =>
    changesHead(
      { vault_uuid: VAULT, through },
      '00000000-0000-4000-8000-000000000002',
    ),
  ).toThrow('变更水位响应格式无效');
});

it('代次号变了不算新变化；提交序号、观察序号、进程标识变了才算', () => {
  expect(sameChanges(through, { ...through, generation: '244' })).toBe(true);
  expect(sameChanges(through, { ...through, sequence: '22' })).toBe(false);
  expect(sameChanges(through, { ...through, observation: '3' })).toBe(false);
  expect(
    sameChanges(through, {
      ...through,
      epoch: 'd11d3d3a-2943-4cae-bf20-000000000002',
    }),
  ).toBe(false);
});
