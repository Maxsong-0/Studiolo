import { expect, it } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { FileOperationStore } from '../fileOperationStore';
import { SessionClient } from '../sessionClient';
import { ownerScope } from './canonicalFixtures';
import {
  deleting,
  fileReceipt,
  restoring,
  tombstone,
} from './fileOperationFixtures';
import { jsonResponse } from './fixtures';
import {
  deferred,
  mockSessions,
  otherSession,
  pair,
  sessionResponse,
} from './sessionFixtures';

it('同键未知请求经真实会话重试：新准入403保留未知，固定409拒绝才收口', async () => {
  for (const [request, code] of [
    [deleting, 'file_operation_conflict'],
    [restoring, 'file_operation_cancelled'],
  ] as const) {
    let attempts = 0;
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/session')
        return jsonResponse({ ...pair, owner_scope: ownerScope });
      if (input !== '/api/v1/file-operations')
        return sessionResponse(input, init);
      attempts += 1;
      if (attempts === 1) throw new TypeError('响应丢失，不证明未执行');
      if (attempts === 2)
        return jsonResponse(
          { error: '当前准入已变化', code: 'forbidden' },
          403,
        );
      return jsonResponse({ error: '原请求已有固定拒绝', code }, 409);
    });
    const client = new SessionClient();
    await client.check();
    const uninstall = installFileOperationWriter(client.fileOperation);
    const store = new FileOperationStore().forScope(ownerScope);
    const signal = new AbortController().signal;
    const source = request.kind === 'restore_tombstone' ? tombstone : undefined;
    try {
      await store.submit(request, signal, source);
      expect(store.getSnapshot()?.phase).toBe('uncertain');
      store.dismiss();
      await store.submit(request, signal);
      expect(store.getSnapshot()).toMatchObject({
        phase: 'uncertain',
        request,
      });
      expect(store.hasPending()).toBe(true);
      if (source) expect(store.getSnapshot()?.restoreSource).toEqual(source);
      await store.submit(request, signal);
      expect(store.getSnapshot()).toMatchObject({ phase: 'rejected', request });
      expect(store.hasPending()).toBe(false);
      const posts = fetchMock.mock.calls.filter(
        ([url]) => url === '/api/v1/file-operations',
      );
      expect(posts).toHaveLength(3);
      expect(posts.map(([, init]) => JSON.parse(String(init?.body)))).toEqual([
        request,
        request,
        request,
      ]);
      store.dismiss();
      expect(store.getSnapshot()).toBeNull();
    } finally {
      uninstall();
      client.stop();
      store.destroy();
    }
  }
});

it('实际文件POST和认证POST双向互斥，被拒绝者不释放持有者busy', async () => {
  const fileResponse = deferred<Response>();
  const authResponse = deferred<Response>();
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/file-operations') return fileResponse.promise;
    if (input === '/api/v1/sessions/revoke') return authResponse.promise;
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  const posts = () =>
    fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST');
  const pending: Promise<unknown>[] = [];
  try {
    const file = client.fileOperation(deleting, new AbortController().signal);
    pending.push(file);
    expect(client.getSnapshot().busy).toBe(true);
    const refusedAuth = client.revoke({
      scope: 'session',
      session_id: otherSession.id,
    });
    pending.push(refusedAuth);
    expect(posts().map(([url]) => url)).toEqual(['/api/v1/file-operations']);
    await expect(refusedAuth).resolves.toBe(false);
    expect(client.getSnapshot().busy).toBe(true);
    fileResponse.resolve(jsonResponse(fileReceipt()));
    await file;
    expect(client.getSnapshot().busy).toBe(false);

    const auth = client.revoke({
      scope: 'session',
      session_id: otherSession.id,
    });
    pending.push(auth);
    expect(client.getSnapshot().busy).toBe(true);
    await expect(
      client.fileOperation(deleting, new AbortController().signal),
    ).rejects.toMatchObject({ status: 429 });
    expect(posts().map(([url]) => url)).toEqual([
      '/api/v1/file-operations',
      '/api/v1/sessions/revoke',
    ]);
    expect(client.getSnapshot().busy).toBe(true);
    authResponse.resolve(jsonResponse({ revoked: 1 }));
    await expect(auth).resolves.toBe(true);
    expect(client.getSnapshot()).toMatchObject({
      phase: 'ready',
      busy: false,
      ownerScope,
    });
  } finally {
    // 失败断言也释放自有 deferred，避免留下测试间的异步任务。
    fileResponse.resolve(jsonResponse(fileReceipt()));
    authResponse.resolve(jsonResponse({ revoked: 1 }));
    client.stop();
    await Promise.allSettled(pending);
  }
});
