import { expect, it } from 'vitest';
import { changesFixture } from '../../dev/fixtures/changes';
import { changesPage, seenReceipt, changeActor } from '../changesModel';
const page = () =>
  changesFixture(
    1794584000000,
    21,
    null,
    '40000000-0000-4000-8000-000000000001',
    false,
  );
it('封闭解码可靠主体、观察缺口和超过 JS 安全整数的水位', () => {
  const raw = page();
  raw.through.sequence = '9007199254740993';
  expect(changesPage(raw, null, null)).toEqual(raw);
  expect(changeActor(raw.entries[0])).toContain('任务');
  expect(changeActor(raw.entries[3])).toBe('外部变化 · 作者未知');
});
it.each(['content', 'payload', 'credentials'])('拒绝顶层多余字段 %s', (key) => {
  expect(() =>
    changesPage({ ...page(), [key]: 'extra' }, null, null),
  ).toThrow();
});
it('拒绝伪作者、未固定未来水位、重复行和来源混淆', () => {
  const malformed = [
    (p: ReturnType<typeof page>) => {
      p.entries[3].actor = { kind: 'obsidian' };
    },
    (p: ReturnType<typeof page>) => {
      p.entries[0].sequence = '999';
    },
    (p: ReturnType<typeof page>) => {
      p.entries.push(p.entries[0]);
    },
    (p: ReturnType<typeof page>) => {
      p.entries[0].generation = '1';
    },
    (p: ReturnType<typeof page>) => {
      p.entries[0].path = '../private';
    },
    (p: ReturnType<typeof page>) => {
      p.entries[3].observation = '99';
    },
    (p: ReturnType<typeof page>) => {
      p.through.sequence = '01';
    },
    (p: ReturnType<typeof page>) => {
      p.entries[0].bytes = '18446744073709551616';
    },
    (p: ReturnType<typeof page>) => {
      p.entries[4].gap = null;
    },
  ];
  for (const mutate of malformed) {
    const raw = page();
    mutate(raw);
    expect(() => changesPage(raw, null, null)).toThrow();
  }
});
it('分页必须保持库、作用域和四项上限，已读回执必须精确对应快照', () => {
  const first = page();
  const next = page();
  next.through.sequence = '22';
  expect(() => changesPage(next, null, first.snapshot, first)).toThrow();
  expect(() => changesPage(first, '项目/Other', null)).toThrow();
  const response = {
    status: 'seen',
    vault_uuid: first.vault_uuid,
    project: first.project,
    through: first.through,
  };
  expect(() => seenReceipt(response, first)).not.toThrow();
  expect(() =>
    seenReceipt(
      { ...response, through: { ...first.through, observation: '999' } },
      first,
    ),
  ).toThrow();
  expect(() => seenReceipt({ ...response, extra: true }, first)).toThrow();
});

it('开发夹具的已读端点也检查 CSRF，并与真实 DTO 同形', async () => {
  const { createFixtureApi } = await import('../../dev/fixtureApi');
  const { CSRF_TOKEN } = await import('../../dev/fixtures/ids');
  const api = createFixtureApi();
  const listed = api.handle({
    method: 'GET',
    url: '/api/v1/changes',
    headers: {},
  });
  const first = changesPage(listed.body, null, null);
  const body = {
    vault_uuid: first.vault_uuid,
    project: null,
    snapshot: first.snapshot,
    request_id: '50000000-0000-4000-8000-000000000001',
  };
  expect(
    api.handle({
      method: 'POST',
      url: '/api/v1/changes/seen',
      headers: {},
      body,
    }).status,
  ).toBe(403);
  const response = api.handle({
    method: 'POST',
    url: '/api/v1/changes/seen',
    headers: { 'x-studiolo-request': '1', 'x-csrf-token': CSRF_TOKEN },
    body,
  });
  expect(response.status).toBe(200);
  expect(() => seenReceipt(response.body, first)).not.toThrow();
});
