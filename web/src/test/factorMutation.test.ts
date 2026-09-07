import { describe, expect, it, vi } from 'vitest';
import { jsonResponse } from './fixtures';
import { csrf } from './sessionFixtures';
import {
  codes,
  factorClient,
  factorResponse,
  mockFactors,
  recent,
} from './factorFixtures';

describe('封闭因子操作', () => {
  it('二次认证使用会话请求规则，返回期限而不保存验证码或额外票据', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    expect(await client.factor({ kind: 'step-up', code: '123456' })).toEqual({
      kind: 'step-up',
      recent,
    });
    const posts = fetchMock.mock.calls.filter(
      ([, init]) => init?.method === 'POST',
    );
    expect(posts).toHaveLength(1);
    expect(posts[0]).toMatchObject([
      '/api/v1/auth/step-up',
      {
        body: '{"code":"123456"}',
        credentials: 'same-origin',
        cache: 'no-store',
        redirect: 'error',
        headers: {
          'X-CSRF-Token': csrf,
          'X-Studiolo-Request': '1',
          'Content-Type': 'application/json',
        },
      },
    ]);
    expect(posts[0][1]?.headers).not.toHaveProperty('Origin');
    expect(posts[0][1]?.headers).not.toHaveProperty('Sec-Fetch-Site');
    expect(JSON.stringify(client.getSnapshot())).not.toContain('123456');
    expect(JSON.stringify(client.getSnapshot())).not.toContain(csrf);
    client.stop();
  });

  it('换发仅发空对象，原码只在本次返回值中交付', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    expect(await client.factor({ kind: 'codes' })).toEqual({
      kind: 'codes',
      codes,
    });
    const request = fetchMock.mock.calls.find(
      ([path]) => path === '/api/v1/auth/recovery-codes',
    )![1];
    expect(request?.body).toBe('{}');
    expect(JSON.stringify(client.getSnapshot())).not.toContain(codes[0]);
    await client.factorState(new AbortController().signal);
    expect(
      fetchMock.mock.calls.filter(
        ([path]) => path === '/api/v1/auth/recovery-codes',
      ),
    ).toHaveLength(1);
    client.stop();
  });

  it('恢复严格接受 204 空响应，广播退出并等待显式登录', async () => {
    const fetchMock = mockFactors();
    const response = new Response(null, { status: 204 });
    const parse = vi.spyOn(response, 'json');
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/factor/recover'
        ? response
        : factorResponse(input, init),
    );
    const client = await factorClient();
    const announce = vi.fn();
    client.onChange(announce);
    expect(
      await client.factor({
        kind: 'recover',
        recovery_code: codes[0],
        code: '123456',
      }),
    ).toEqual({ kind: 'recovered' });
    expect(parse).not.toHaveBeenCalled();
    expect(announce).toHaveBeenCalledOnce();
    expect(client.getSnapshot()).toMatchObject({
      phase: 'signed-out',
      session: null,
    });
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    client.stop();
  });

  it.each([403, 409, 429])(
    '%s 清输入语义与能力快照，保留 L2 且不重放',
    async (status) => {
      const fetchMock = mockFactors();
      fetchMock.mockImplementation(async (input, init) =>
        input === '/api/v1/auth/step-up'
          ? jsonResponse({ error: 'do not expose server internals' }, status)
          : factorResponse(input, init),
      );
      const client = await factorClient();
      const result = await client.factor({ kind: 'step-up', code: '123456' });
      expect(result).toMatchObject({ kind: 'failed', status });
      expect(JSON.stringify(result)).not.toContain('server internals');
      expect(client.getSnapshot()).toMatchObject({
        phase: 'ready',
        busy: false,
      });
      await client.factor({ kind: 'step-up', code: '654321' });
      expect(
        fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
      ).toHaveLength(1);
      await client.factorState(new AbortController().signal);
      await client.factor({ kind: 'step-up', code: '654321' });
      expect(
        fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
      ).toHaveLength(2);
      client.stop();
    },
  );

  it.each(['503', 'network', 'malformed'])(
    '%s 的换码交付不能判定失败，状态核对不抹去提示',
    async (failure) => {
      const fetchMock = mockFactors();
      fetchMock.mockImplementation(async (input, init) => {
        if (input !== '/api/v1/auth/recovery-codes')
          return factorResponse(input, init);
        if (failure === 'network') throw new TypeError('connection lost');
        return failure === '503'
          ? jsonResponse({}, 503)
          : jsonResponse({ codes: [...codes.slice(1), codes[1]] });
      });
      const client = await factorClient();
      await client.factor({ kind: 'codes' });
      expect(client.getSnapshot()).toMatchObject({
        phase: 'uncertain',
        session: null,
      });
      expect(client.getSnapshot().factorNotice).toContain('原码无法重读');
      await client.factor({ kind: 'codes' });
      await client.check();
      await client.factorState(new AbortController().signal);
      expect(client.getSnapshot().phase).toBe('ready');
      expect(client.getSnapshot().factorNotice).toContain('独立的新操作');
      expect(
        fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
      ).toHaveLength(1);
      client.stop();
    },
  );

  it('恢复收到意外 200 不能宣布成功或自动登录', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/factor/recover'
        ? jsonResponse({})
        : factorResponse(input, init),
    );
    const client = await factorClient();
    expect(
      await client.factor({
        kind: 'recover',
        recovery_code: codes[0],
        code: '123456',
      }),
    ).toEqual({ kind: 'stopped' });
    expect(client.getSnapshot()).toMatchObject({
      phase: 'uncertain',
      session: null,
    });
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    client.stop();
  });
});
