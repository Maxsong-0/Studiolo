import { expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { installChangesWriter, markChangesSeen } from '../changesApi';
import { changesFixture } from '../../dev/fixtures/changes';
import { csrf, mockSessions, pair, sessionResponse } from './sessionFixtures';
import { ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';
it('已读 POST 走唯一会话与 CSRF，401 清空主人内容权限', async () => {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (path, init) => {
    if (path === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (path === '/api/v1/changes/seen')
      return jsonResponse({ error: '登录已失效' }, 401);
    return sessionResponse(path, init);
  });
  const client = new SessionClient();
  await client.check();
  const uninstall = installChangesWriter(client.changesSeen);
  try {
    const page = changesFixture(
      Date.now(),
      21,
      null,
      '40000000-0000-4000-8000-000000000001',
      false,
    );
    await expect(
      markChangesSeen(
        page,
        '50000000-0000-4000-8000-000000000001',
        new AbortController().signal,
      ),
    ).rejects.toThrow();
    const call = fetchMock.mock.calls.find(
      ([path]) => path === '/api/v1/changes/seen',
    )!;
    expect(new Headers(call[1]?.headers).get('x-csrf-token')).toBe(csrf);
    expect(client.getSnapshot().phase).toBe('problem');
    expect(client.getSnapshot().ownerScope).toBeNull();
  } finally {
    uninstall();
    client.stop();
  }
});
