import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { usePageHistory } from '../usePageHistory';
import { historyEntry, historyFile, historyPage } from './historyFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';

it('翻页立即清选择，忽略取消的旧正文；history_changed 只允许重新第一页', async () => {
  const body = deferred<Response>();
  const fetchMock = vi.fn(async (url: string) => {
    if (url.startsWith('/api/v1/history/file?')) return body.promise;
    if (new URL(url, location.origin).searchParams.has('cursor'))
      return jsonResponse({ code: 'history_changed', error: '已改变' }, 409);
    return jsonResponse({ ...historyPage, next_cursor: 'opaque-next' });
  });
  vi.stubGlobal('fetch', fetchMock);
  const { result } = renderHook(() => usePageHistory(historyPage));
  await waitFor(() => expect(result.current.page).not.toBeNull());
  let selecting!: Promise<void>;
  act(() => {
    selecting = result.current.select(historyEntry);
  });
  await act(async () => result.current.readPage('opaque-next'));
  expect(result.current).toMatchObject({
    page: null,
    selected: null,
    file: null,
    changed: true,
  });
  await act(async () => {
    body.resolve(jsonResponse(historyFile));
    await selecting;
  });
  expect(result.current.file).toBeNull();
  await act(async () => result.current.readPage(null));
  expect(result.current.page).not.toBeNull();
  expect(
    new URL(fetchMock.mock.calls.at(-1)![0], location.origin).searchParams.has(
      'cursor',
    ),
  ).toBe(false);
});
it('路径切换与卸载使在途历史失效，范围 403 不回退为更宽查询', async () => {
  const first = deferred<Response>();
  const fetchMock = vi.fn(async (url: string) => {
    if (
      new URL(url, location.origin).searchParams.get('path') ===
      historyPage.path
    )
      return first.promise;
    return jsonResponse({ code: 'protected', error: '范围不允许' }, 403);
  });
  vi.stubGlobal('fetch', fetchMock);
  const { result, rerender, unmount } = renderHook(
    ({ path }) => usePageHistory({ vault_uuid: historyPage.vault_uuid, path }),
    { initialProps: { path: historyPage.path } },
  );
  rerender({ path: '别的/页面.md' });
  await waitFor(() => expect(result.current.error).toBe('范围不允许'));
  await act(async () => first.resolve(jsonResponse(historyPage)));
  expect(result.current.page).toBeNull();
  expect(fetchMock).toHaveBeenCalledTimes(2);
  unmount();
});
it('旧服务 404 安全只读降级；损坏 present 响应不能伪装空历史', async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse({}, 404))
    .mockResolvedValueOnce(jsonResponse({ ...historyPage, entries: '损坏' }));
  vi.stubGlobal('fetch', fetchMock);
  const { result } = renderHook(() => usePageHistory(historyPage));
  await waitFor(() => expect(result.current.unavailable).toBe(true));
  expect(result.current.error).toContain('不能据此恢复');
  await act(async () => result.current.readPage(null));
  expect(result.current).toMatchObject({ page: null, unavailable: false });
  expect(result.current.error).toContain('格式无效');
});
it('历史版本已清理不冒称旧服务；撤权后清除已显示的历史元数据', async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(historyPage))
    .mockResolvedValueOnce(
      jsonResponse({ code: 'history_not_found', error: '版本已清理' }, 404),
    )
    .mockResolvedValueOnce(
      jsonResponse({ code: 'protected', error: '读取范围已撤销' }, 403),
    );
  vi.stubGlobal('fetch', fetchMock);
  const { result } = renderHook(() => usePageHistory(historyPage));
  await waitFor(() => expect(result.current.page).not.toBeNull());
  await act(async () => result.current.select(historyEntry));
  expect(result.current).toMatchObject({
    unavailable: false,
    selected: null,
    file: null,
    error: '版本已清理',
  });
  await act(async () => result.current.select(historyEntry));
  expect(result.current).toMatchObject({
    page: null,
    selected: null,
    file: null,
    error: '读取范围已撤销',
  });
});
