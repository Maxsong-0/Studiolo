import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useWorkbench } from '../useWorkbench';
import { fixture, jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';

it('refreshLatest只返回本次实际发布的数据，后来的同代请求使旧请求返回null', async () => {
  const old = deferred<Response>();
  const latest = deferred<Response>();
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(fixture))
    .mockReturnValueOnce(old.promise)
    .mockReturnValueOnce(latest.promise);
  vi.stubGlobal('fetch', fetcher);
  const { result } = renderHook(useWorkbench);
  await waitFor(() => expect(result.current.data).not.toBeNull());
  let one!: Promise<typeof fixture | null>;
  let two!: Promise<typeof fixture | null>;
  act(() => {
    one = result.current.refreshLatest();
    two = result.current.refreshLatest();
  });
  const accepted = { ...fixture, source_label: '本次真实新快照' };
  await act(async () => {
    latest.resolve(jsonResponse(accepted));
    expect(await two).toEqual(accepted);
  });
  await act(async () => {
    old.resolve(jsonResponse({ ...fixture, source_label: '迟到快照' }));
    expect(await one).toBeNull();
  });
  expect(result.current.data?.source_label).toBe('本次真实新快照');
  expect(fetcher.mock.calls[1][0]).toBe('/api/workbench?refresh=true');
});
it('失败和卸载后返回null；失败时旧data只是带错误的上次快照', async () => {
  const late = deferred<Response>();
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockResolvedValueOnce(jsonResponse(fixture))
      .mockRejectedValueOnce(new TypeError('offline'))
      .mockReturnValueOnce(late.promise),
  );
  const { result, unmount } = renderHook(useWorkbench);
  await waitFor(() => expect(result.current.data).not.toBeNull());
  await act(async () => {
    expect(await result.current.refreshLatest()).toBeNull();
  });
  expect(result.current.data).toEqual(fixture);
  expect(result.current.error).toContain('offline');
  let value!: Promise<typeof fixture | null>;
  act(() => {
    value = result.current.refreshLatest();
  });
  unmount();
  late.resolve(jsonResponse(fixture));
  expect(await value).toBeNull();
});
