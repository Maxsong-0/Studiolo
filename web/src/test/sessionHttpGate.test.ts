import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AUTH_SLOTS, requestJson } from '../sessionHttp';
import { deferred } from './sessionFixtures';
import { jsonResponse } from './fixtures';

let pending: Map<string, ReturnType<typeof deferred<Response>>>;
let sent: string[];
let controllers: AbortController[];

/** 闸门是模块级的：每例结束都取消本例的请求，让名额全部归还，不漏给下一例。 */
function controller() {
  const made = new AbortController();
  controllers.push(made);
  return made;
}

beforeEach(() => {
  pending = new Map();
  sent = [];
  controllers = [];
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL) => {
      const path = String(input);
      sent.push(path);
      const answer = deferred<Response>();
      pending.set(path, answer);
      return answer.promise;
    }),
  );
});
afterEach(() => {
  for (const made of controllers) made.abort();
  vi.unstubAllGlobals();
});

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('认证请求闸门', () => {
  it('在途最多两个，其余按到达顺序排队，前面的一返回就放下一个', async () => {
    expect(AUTH_SLOTS).toBe(2);
    const signal = controller().signal;
    const results = ['/a', '/b', '/c', '/d'].map((path) =>
      requestJson(path, signal),
    );
    await tick();
    expect(sent).toEqual(['/a', '/b']);
    pending.get('/b')!.resolve(jsonResponse({ ok: 'b' }));
    await tick();
    expect(sent).toEqual(['/a', '/b', '/c']);
    pending.get('/a')!.resolve(jsonResponse({ ok: 'a' }));
    await tick();
    expect(sent).toEqual(['/a', '/b', '/c', '/d']);
    pending.get('/c')!.resolve(jsonResponse({ ok: 'c' }));
    pending.get('/d')!.resolve(jsonResponse({ ok: 'd' }));
    await expect(Promise.all(results)).resolves.toEqual([
      { ok: 'a' },
      { ok: 'b' },
      { ok: 'c' },
      { ok: 'd' },
    ]);
  });

  it('bootstrap 不校验会话，不占名额也不排队', async () => {
    const signal = controller().signal;
    void requestJson('/a', signal).catch(() => undefined);
    void requestJson('/b', signal).catch(() => undefined);
    void requestJson('/api/v1/bootstrap', signal).catch(() => undefined);
    await tick();
    // 两个名额已满，bootstrap 仍然照发。
    expect([...sent].sort()).toEqual(['/a', '/api/v1/bootstrap', '/b']);
  });

  it('排队中被取消的请求不会发出；在途的被取消立刻让出名额', async () => {
    const keep = controller();
    const first = controller();
    const queued = controller();
    void requestJson('/a', first.signal).catch(() => undefined);
    void requestJson('/b', keep.signal);
    const dropped = requestJson('/c', queued.signal);
    const later = requestJson('/d', keep.signal);
    await tick();
    queued.abort();
    await expect(dropped).rejects.toMatchObject({ name: 'AbortError' });
    // 在途的 /a 被取消：夹具里的 fetch 不理会取消，名额也必须当场让出。
    first.abort();
    await tick();
    expect(sent).toEqual(['/a', '/b', '/d']);
    pending.get('/b')!.resolve(jsonResponse({}));
    pending.get('/d')!.resolve(jsonResponse({ ok: 'd' }));
    await expect(later).resolves.toEqual({ ok: 'd' });
  });
});
