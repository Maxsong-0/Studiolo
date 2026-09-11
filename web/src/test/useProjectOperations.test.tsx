import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useProjectOperations } from '../useProjectOperations';
import { jsonResponse } from './fixtures';
import { operationsData } from './operationsFixtures';

it('切换项目立即丢弃旧结果，取消的迟到请求不覆盖新项目', async () => {
  let finishOld!: (response: Response) => void;
  const fetchMock = vi.fn<typeof fetch>();
  fetchMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve;
      }),
  );
  fetchMock.mockResolvedValueOnce(
    jsonResponse(operationsData({ project: '项目/新项目' })),
  );
  vi.stubGlobal('fetch', fetchMock);
  const { result, rerender } = renderHook(
    ({ project }) => useProjectOperations(project, 7),
    {
      initialProps: { project: '项目/星舟' },
    },
  );
  await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
  rerender({ project: '项目/新项目' });
  expect(result.current.result).toBeNull();
  expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(true);
  await waitFor(() =>
    expect(result.current.result?.project).toBe('项目/新项目'),
  );
  await act(async () => finishOld(jsonResponse(operationsData())));
  expect(result.current.result?.project).toBe('项目/新项目');
});

it('文档代次更新隐藏旧声明，代次不匹配按409刷新处理', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(async () => jsonResponse(operationsData()));
  vi.stubGlobal('fetch', fetchMock);
  const { result, rerender } = renderHook(
    ({ generation }) => useProjectOperations('项目/星舟', generation),
    {
      initialProps: { generation: 7 },
    },
  );
  await waitFor(() => expect(result.current.result?.generation).toBe(7));
  rerender({ generation: 8 });
  expect(result.current.result).toBeNull();
  await waitFor(() => expect(result.current.stale).toBe(true));
  expect(result.current.error).toContain('不同快照');
  fetchMock.mockResolvedValueOnce(
    jsonResponse(operationsData({ generation: 8 })),
  );
  act(() => result.current.retry());
  await waitFor(() => expect(result.current.result?.generation).toBe(8));
});

it('响应项目不一致时拒绝显示其他项目的声明', async () => {
  vi.stubGlobal(
    'fetch',
    vi
      .fn()
      .mockResolvedValue(
        jsonResponse(operationsData({ project: '项目/其他项目' })),
      ),
  );
  const { result } = renderHook(() => useProjectOperations('项目/星舟', 7));
  await waitFor(() =>
    expect(result.current.error).toContain('与所选项目不一致'),
  );
  expect(result.current.result).toBeNull();
  expect(result.current.stale).toBe(false);
});
