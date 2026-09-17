import { expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { FileOperationStore } from '../fileOperationStore';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming, renameResponse } from './renameFixtures';
import { ownerScope, replacement } from './canonicalFixtures';
import {
  csrf,
  deferred,
  mockSessions,
  pair,
  sessionResponse,
} from './sessionFixtures';
import { jsonResponse } from './fixtures';

it('Rename使用原会话CSRF和唯一contentPending，不夹带path或绕过保存互斥', async () => {
  const response = deferred<Response>();
  const fetcher = mockSessions();
  fetcher.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/file-operations') return response.promise;
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  const uninstall = installFileOperationWriter(client.fileOperation);
  const store = new FileOperationStore().forScope(ownerScope);
  const sending = store.submit(renaming, new AbortController().signal);
  try {
    await expect(
      client.replace(replacement, new AbortController().signal),
    ).rejects.toMatchObject({ status: 429 });
    const posts = fetcher.mock.calls.filter(
      ([, init]) => init?.method === 'POST',
    );
    expect(posts).toHaveLength(1);
    expect(posts[0][0]).toBe('/api/v1/file-operations');
    expect(JSON.parse(String(posts[0][1]?.body))).toEqual(renaming);
    expect(new Headers(posts[0][1]?.headers).get('X-CSRF-Token')).toBe(csrf);
    response.resolve(jsonResponse(renameResponse()));
    await sending;
    expect(store.getSnapshot()?.phase).toBe('committed');
  } finally {
    response.resolve(jsonResponse(renameResponse()));
    await sending;
    uninstall();
    client.stop();
  }
});
