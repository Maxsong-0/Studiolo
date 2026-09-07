import { describe, expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import {
  factorClient,
  factorResponse,
  factorSnapshot,
  mockFactors,
  readyFactors,
} from './factorFixtures';

describe('因子能力快照', () => {
  it('只有明确的因子状态才开放操作，登录本身不探测或提交因子', async () => {
    const fetchMock = mockFactors();
    const client = new SessionClient();
    await client.check();
    expect(
      await client.factor({ kind: 'step-up', code: '123456' }),
    ).toMatchObject({ kind: 'failed' });
    expect(fetchMock.mock.calls.map(([path]) => path)).toEqual([
      '/api/v1/bootstrap',
      '/api/v1/session',
    ]);
    await client.factorState(new AbortController().signal);
    expect(
      await client.factor({ kind: 'step-up', code: '123456' }),
    ).toMatchObject({ kind: 'step-up' });
    client.stop();
  });

  it('明确未装配因子保持 L2，但没有因子写入口', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/state'
        ? jsonResponse({ factors: null })
        : factorResponse(input, init),
    );
    const client = new SessionClient();
    await client.check();
    expect(await client.factorState(new AbortController().signal)).toBeNull();
    expect(await client.factor({ kind: 'codes' })).toMatchObject({
      kind: 'failed',
    });
    expect(client.getSnapshot().phase).toBe('ready');
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
    ).toBe(false);
    client.stop();
  });

  it.each([
    { name: '旧入口 404', response: () => jsonResponse({}, 404) },
    {
      name: '未知字段',
      response: () => jsonResponse({ factors: null, available: true }),
    },
    { name: '缺失字段', response: () => jsonResponse({ factors: {} }) },
    {
      name: '未来认证时间',
      response: () =>
        jsonResponse({ factors: { ...readyFactors, observed_at_ms: 0 } }),
    },
    {
      name: '非布尔候选',
      response: () =>
        jsonResponse({
          factors: { ...factorSnapshot, recovery_candidate_available: 'yes' },
        }),
    },
  ])('$name 不能被解释为已知能力或认证降级', async ({ response }) => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/state' ? response() : factorResponse(input, init),
    );
    await expect(
      client.factorState(new AbortController().signal),
    ).rejects.toThrow();
    expect(await client.factor({ kind: 'codes' })).toMatchObject({
      kind: 'failed',
    });
    expect(client.getSnapshot().phase).toBe('ready');
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
    ).toBe(false);
    client.stop();
  });

  it('迟到的旧状态不能覆盖后续明确未启用的能力', async () => {
    const fetchMock = mockFactors();
    const client = await factorClient();
    const old = deferred<Response>();
    fetchMock
      .mockReturnValueOnce(old.promise)
      .mockResolvedValueOnce(jsonResponse({ factors: null }));
    const stale = client
      .factorState(new AbortController().signal)
      .catch((error: unknown) => error);
    expect(await client.factorState(new AbortController().signal)).toBeNull();
    old.resolve(jsonResponse({ factors: readyFactors }));
    expect(await stale).toMatchObject({ name: 'AbortError' });
    expect(await client.factor({ kind: 'codes' })).toMatchObject({
      kind: 'failed',
    });
    client.stop();
  });

  it('pending 只是警示，显式核对后允许独立的新认证操作', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/state'
        ? jsonResponse({ factors: { ...factorSnapshot, pending: true } })
        : factorResponse(input, init),
    );
    const client = await factorClient();
    expect(
      await client.factor({ kind: 'step-up', code: '654321' }),
    ).toMatchObject({ kind: 'step-up' });
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    client.stop();
  });

  it('拒绝非规范输入及没有准备候选的恢复，不发探测 POST', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/state'
        ? jsonResponse({
            factors: { ...factorSnapshot, recovery_candidate_available: false },
          })
        : factorResponse(input, init),
    );
    const client = await factorClient();
    for (const code of [' 123456', '１２３４５６', '12345', '1234567']) {
      expect(await client.factor({ kind: 'step-up', code })).toMatchObject({
        kind: 'failed',
      });
    }
    expect(
      await client.factor({
        kind: 'recover',
        recovery_code: 'invalid',
        code: '123456',
      }),
    ).toMatchObject({ kind: 'failed' });
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
    ).toBe(false);
    client.stop();
  });
});
