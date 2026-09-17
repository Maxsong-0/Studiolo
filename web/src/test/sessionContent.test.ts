import { expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { sessionPair } from '../sessionModel';
import {
  canonical,
  committed,
  ownerScope,
  replacement,
} from './canonicalFixtures';
import {
  csrf,
  deferred,
  mockSessions,
  pair,
  sessionResponse,
} from './sessionFixtures';
import { jsonResponse } from './fixtures';

it('owner scope 旧响应只读，present 格式错误不能降级', () => {
  expect(sessionPair(pair).ownerScope).toBeNull();
  for (const owner_scope of [null, [], ['a'.repeat(64)], 'x', 'F'.repeat(64)]) {
    expect(() => sessionPair({ ...pair, owner_scope })).toThrow();
  }
});
it('业务保存只走已验证会话的固定路由和 CSRF，保留结构化 409', async () => {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/mutations')
      return jsonResponse(
        {
          code: 'revision_conflict',
          error: '冲突',
          vault_uuid: canonical.vault_uuid,
          path: canonical.path,
          expected_revision: canonical.revision,
          actual_revision: null,
        },
        409,
      );
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  await expect(
    client.replace(replacement, new AbortController().signal),
  ).rejects.toMatchObject({
    status: 409,
    details: { code: 'revision_conflict', actual_revision: null },
  });
  const call = fetchMock.mock.calls.find(
    ([path]) => path === '/api/v1/mutations',
  )!;
  expect(call[1]).toMatchObject({
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'X-CSRF-Token': csrf },
  });
  expect(JSON.parse(String(call[1]?.body))).toEqual(replacement);
  expect(JSON.stringify(client.getSnapshot())).not.toContain(csrf);
  client.stop();
});
it('缺 owner scope 不发出 POST', async () => {
  const fetchMock = mockSessions();
  const client = new SessionClient();
  await client.check();
  await expect(
    client.replace(replacement, new AbortController().signal),
  ).rejects.toMatchObject({ status: 403 });
  expect(
    fetchMock.mock.calls.some(([path]) => path === '/api/v1/mutations'),
  ).toBe(false);
  client.stop();
});
it('401 隐去授权，忽略取消的旧成功不能跨会话恢复保存状态', async () => {
  const fetchMock = mockSessions();
  const pending = deferred<Response>();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/mutations') return pending.promise;
    if (input === '/expired') return jsonResponse({}, 401);
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  const saving = client
    .replace(replacement, new AbortController().signal)
    .catch((error: unknown) => error);
  await expect(
    client.read('/expired', new AbortController().signal),
  ).rejects.toMatchObject({ status: 401 });
  expect(client.getSnapshot()).toMatchObject({
    phase: 'problem',
    ownerScope: null,
  });
  await client.check();
  pending.resolve(jsonResponse(committed(replacement)));
  expect(await saving).toMatchObject({ name: 'AbortError' });
  client.stop();
});
