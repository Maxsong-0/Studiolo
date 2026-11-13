import { Blob as NodeBlob } from 'node:buffer';
import { afterEach, expect, it, vi } from 'vitest';
import { ApiError, requestBlob, requestJson } from '../sessionHttp';
import { SessionClient } from '../sessionClient';
import { deferred, mockSessions } from './sessionFixtures';

const response = (body = 'abc', size = 3) =>
  new Response(body, {
    headers: { 'Content-Length': String(size), 'Content-Type': 'image/png' },
  });
afterEach(() => vi.unstubAllGlobals());

it('二进制和 JSON 请求共用两个认证名额', async () => {
  vi.stubGlobal('Blob', NodeBlob);
  const first = deferred<Response>();
  const second = deferred<Response>();
  const fetchMock = vi
    .fn()
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise)
    .mockResolvedValue(response());
  vi.stubGlobal('fetch', fetchMock);
  const signal = new AbortController().signal;
  const a = requestBlob('/a', signal, 3);
  const b = requestJson('/b', signal);
  const c = requestBlob('/c', signal, 3);
  expect(fetchMock).toHaveBeenCalledTimes(2);
  second.resolve(new Response('{}'));
  await b;
  await c;
  expect(fetchMock).toHaveBeenCalledTimes(3);
  first.resolve(response());
  await a;
});

it('声明大小相符仍核实际字节数，流过长和过短均拒绝', async () => {
  for (const body of ['ab', 'abcd']) {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(response(body)));
    await expect(
      requestBlob('/attachment', new AbortController().signal, 3),
    ).rejects.toThrow(/附件/);
  }
});

it('附件 401 使同一会话失效，迟到字节不能恢复内容', async () => {
  vi.stubGlobal('Blob', NodeBlob);
  mockSessions();
  const client = new SessionClient();
  await client.check();
  const pending = deferred<Response>();
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValueOnce(new Response('', { status: 401 })),
  );
  const slow = client
    .readBlob('/slow', new AbortController().signal, 3)
    .catch((error: unknown) => error);
  await expect(
    client.readBlob('/expired', new AbortController().signal, 3),
  ).rejects.toBeInstanceOf(ApiError);
  expect(client.getSnapshot().phase).toBe('problem');
  pending.resolve(response());
  expect(await slow).toMatchObject({ name: 'AbortError' });
  client.stop();
});
