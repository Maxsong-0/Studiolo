import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useSearch } from '../useSearch';
import { jsonResponse } from './fixtures';

it('200ms防抖合并输入，已取消的迟到结果不覆盖最新查询', async () => {
  let finishOld!: (response: Response) => void;
  const fetchMock = vi.fn<typeof fetch>();
  fetchMock.mockImplementationOnce(
    () =>
      new Promise<Response>((resolve) => {
        finishOld = resolve;
      }),
  );
  fetchMock.mockResolvedValueOnce(
    jsonResponse({
      generation: 3,
      hits: [
        { path: 'new.md', title: '新结果', line: 1, snippet: '', kind: 'path' },
      ],
      truncated: false,
    }),
  );
  vi.stubGlobal('fetch', fetchMock);
  const { result, rerender } = renderHook(
    ({ query }) => useSearch(query, 'path', '', 3),
    { initialProps: { query: '初始' } },
  );
  rerender({ query: '旧查询' });
  await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
  expect(String(fetchMock.mock.calls[0][0])).toContain(
    new URLSearchParams({ q: '旧查询' }).toString(),
  );
  rerender({ query: '新查询' });
  expect(result.current.result).toBeNull();
  expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(true);
  await waitFor(() =>
    expect(result.current.result?.hits[0].title).toBe('新结果'),
  );
  await act(async () =>
    finishOld(
      jsonResponse({
        generation: 3,
        hits: [
          {
            path: 'old.md',
            title: '旧结果',
            line: 1,
            snippet: '',
            kind: 'path',
          },
        ],
        truncated: false,
      }),
    ),
  );
  expect(result.current.result?.hits[0].title).toBe('新结果');
});
