import { describe, expect, it } from 'vitest';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import {
  codes,
  factorClient,
  factorResponse,
  mockFactors,
} from './factorFixtures';

describe('因子请求与会话共用寿命', () => {
  it('取消等待立即关闭内容，忽略取消的迟到恢复码不能交付', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    const old = deferred<Response>();
    fetchMock.mockReturnValueOnce(old.promise);
    const controller = new AbortController();
    const pending = client.factor({ kind: 'codes' }, controller.signal);
    controller.abort();
    expect(client.getSnapshot()).toMatchObject({
      phase: 'uncertain',
      session: null,
    });
    expect(client.getSnapshot().factorNotice).toContain('恢复码可能已经换发');
    old.resolve(jsonResponse({ codes }));
    expect(await pending).toEqual({ kind: 'stopped' });
    expect(JSON.stringify(client.getSnapshot())).not.toContain(codes[0]);
    client.stop();
  });

  it('因子操作与吊销、检查、重复点击共享 POST 互斥', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    const old = deferred<Response>();
    fetchMock.mockReturnValueOnce(old.promise);
    const pending = client.factor({ kind: 'codes' });
    expect(await client.factor({ kind: 'codes' })).toEqual({ kind: 'stopped' });
    expect(await client.revoke({ scope: 'all' })).toBe(false);
    await client.check();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    old.resolve(jsonResponse({ codes }));
    expect(await pending).toEqual({ kind: 'codes', codes });
    expect(client.getSnapshot()).toMatchObject({ phase: 'ready', busy: false });
    client.stop();
  });

  it('并发 401 后即使重新登录核对成功，旧因子响应也不能复活', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    const old = deferred<Response>();
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/auth/recovery-codes') return old.promise;
      if (input === '/expired') return jsonResponse({}, 401);
      return factorResponse(input, init);
    });
    const pending = client.factor({ kind: 'codes' });
    await expect(
      client.read('/expired', new AbortController().signal),
    ).rejects.toMatchObject({ status: 401 });
    await client.check();
    old.resolve(jsonResponse({ codes }));
    expect(await pending).toEqual({ kind: 'stopped' });
    expect(client.getSnapshot()).toMatchObject({ phase: 'ready', busy: false });
    expect(client.getSnapshot().factorNotice).toContain('原码无法重读');
    expect(await client.factor({ kind: 'codes' })).toMatchObject({
      kind: 'failed',
    });
    client.stop();
  });

  it.each(['suspend', 'changed'] as const)(
    '%s 不自动核对或重放已提交动作',
    async (event) => {
      const fetchMock = mockFactors();
      const client = await factorClient();
      const old = deferred<Response>();
      fetchMock.mockReturnValueOnce(old.promise);
      const pending = client.factor({ kind: 'step-up', code: '123456' });
      client[event]();
      expect(client.getSnapshot().phase).toBe('uncertain');
      const count = fetchMock.mock.calls.length;
      client.resume();
      expect(fetchMock).toHaveBeenCalledTimes(count);
      old.resolve(factorResponse('/api/v1/auth/step-up'));
      expect(await pending).toEqual({ kind: 'stopped' });
      expect(client.getSnapshot().phase).toBe('uncertain');
      client.stop();
    },
  );
});
