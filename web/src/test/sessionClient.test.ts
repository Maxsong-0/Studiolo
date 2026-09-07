import { describe, expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { jsonResponse } from './fixtures';
import {
  csrf,
  currentSession,
  deferred,
  mockSessions,
  pair,
  sessionResponse,
} from './sessionFixtures';

describe('内存会话生命周期', () => {
  it('并发 401 清理后，旧成功响应即使忽略取消也不能复活授权或内容', async () => {
    const fetchMock = mockSessions();
    const client = new SessionClient();
    await client.check();
    const stale = deferred<Response>();
    fetchMock.mockImplementation((input, init) => {
      if (input === '/slow') return stale.promise;
      if (input === '/expired') return Promise.resolve(jsonResponse({}, 401));
      return Promise.resolve(sessionResponse(input, init));
    });
    const old = client
      .read('/slow', new AbortController().signal)
      .catch((error: unknown) => error);
    await expect(
      client.read('/expired', new AbortController().signal),
    ).rejects.toMatchObject({ status: 401 });
    expect(client.getSnapshot().phase).toBe('problem');
    await client.check();
    stale.resolve(jsonResponse({ obsolete: true }));
    expect(await old).toMatchObject({ name: 'AbortError' });
    expect(client.getSnapshot().phase).toBe('ready');
    client.stop();
  });

  it('迟到的登录响应遇到跨标签变化后停在结果不确定，不自动核对或重试', async () => {
    const fetchMock = mockSessions();
    const login = deferred<Response>();
    fetchMock.mockImplementation((input, init) => {
      if (input === '/api/v1/session')
        return Promise.resolve(jsonResponse({}, 401));
      if (init?.method === 'POST') return login.promise;
      return Promise.resolve(sessionResponse(input, init));
    });
    const client = new SessionClient();
    await client.check();
    const pending = client.login();
    client.changed();
    expect(client.getSnapshot().phase).toBe('uncertain');
    const count = fetchMock.mock.calls.length;
    client.resume();
    expect(fetchMock).toHaveBeenCalledTimes(count);
    login.resolve(jsonResponse(pair, 201));
    await pending;
    expect(client.getSnapshot().session).toBeNull();
    expect(client.getSnapshot().phase).toBe('uncertain');
    client.stop();
  });

  it('旧引导响应不能在暂停与新检查后覆盖新的认证模式', async () => {
    const fetchMock = mockSessions();
    const old = deferred<Response>();
    fetchMock.mockReturnValueOnce(old.promise);
    const client = new SessionClient();
    const pending = client.check();
    client.suspend();
    await client.check();
    expect(client.getSnapshot().phase).toBe('ready');
    old.resolve(jsonResponse({ authentication: 'local' }));
    await pending;
    expect(client.getSnapshot().phase).toBe('ready');
    client.stop();
  });

  it('网络中断的 POST 不重试，显式核对自身后才允许下一次登录', async () => {
    const fetchMock = mockSessions();
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/session') return jsonResponse({}, 401);
      if (init?.method === 'POST') throw new TypeError('Failed to fetch');
      return sessionResponse(input, init);
    });
    const client = new SessionClient();
    await client.check();
    await client.login();
    expect(client.getSnapshot().phase).toBe('uncertain');
    await client.login();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    await client.check();
    expect(client.getSnapshot().phase).toBe('signed-out');
    client.stop();
  });

  it('吊销结果 503 隐去内容与 CSRF，显式核对已吊销状态', async () => {
    const fetchMock = mockSessions();
    const client = new SessionClient();
    await client.check();
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/sessions/revoke') return jsonResponse({}, 503);
      if (input === '/api/v1/session') return jsonResponse({}, 401);
      return sessionResponse(input, init);
    });
    expect(await client.revoke({ scope: 'all' })).toBe(false);
    expect(client.getSnapshot()).toMatchObject({
      phase: 'uncertain',
      session: null,
    });
    await client.revoke({ scope: 'all' });
    await client.check();
    expect(client.getSnapshot().phase).toBe('signed-out');
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    client.stop();
  });

  it('过期或畸形会话响应失败关闭，快照不包含凭据', async () => {
    const fetchMock = mockSessions();
    const client = new SessionClient();
    await client.check();
    expect(JSON.stringify(client.getSnapshot())).not.toContain(csrf);
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/session'
        ? jsonResponse({
            ...pair,
            session: { ...currentSession, state: 'expired' },
          })
        : sessionResponse(input, init),
    );
    await client.check();
    expect(client.getSnapshot()).toMatchObject({
      phase: 'problem',
      session: null,
    });
    await expect(
      client.read('/api/workbench', new AbortController().signal),
    ).rejects.toMatchObject({ name: 'AbortError' });
    client.stop();
  });

  it('重复点击只签发一次，检查与 POST 不并行', async () => {
    const fetchMock = mockSessions();
    const pending = deferred<Response>();
    fetchMock.mockImplementation((input, init) =>
      input === '/api/v1/session'
        ? Promise.resolve(jsonResponse({}, 401))
        : init?.method === 'POST'
          ? pending.promise
          : Promise.resolve(sessionResponse(input, init)),
    );
    const client = new SessionClient();
    await client.check();
    const login = client.login();
    await client.login();
    await client.check();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    pending.resolve(jsonResponse(pair, 201));
    await login;
    expect(client.getSnapshot().phase).toBe('ready');
    client.stop();
  });
});
