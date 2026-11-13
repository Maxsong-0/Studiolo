import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useChanges } from '../useChanges';
import { installChangesWriter } from '../changesApi';
import { installSessionReader, ApiError } from '../sessionHttp';
import { changesFixture } from '../../dev/fixtures/changes';
import { deferred } from './sessionFixtures';
const snapshot = '40000000-0000-4000-8000-000000000001';
it('作用域切换取消在途读取，旧结果不能覆盖新项目', async () => {
  const old = deferred<unknown>();
  const uninstall = installSessionReader(async (url) =>
    new URL(url, location.origin).searchParams.has('project')
      ? changesFixture(Date.now(), 21, '项目/Other', snapshot, false)
      : old.promise,
  );
  try {
    const { result, rerender } = renderHook(
      ({ project }) => useChanges(project),
      { initialProps: { project: null as string | null } },
    );
    rerender({ project: '项目/Other' });
    await waitFor(() =>
      expect(result.current.page?.project).toBe('项目/Other'),
    );
    await act(async () =>
      old.resolve(changesFixture(Date.now(), 21, null, snapshot, false)),
    );
    expect(result.current.page?.project).toBe('项目/Other');
  } finally {
    uninstall();
  }
});
it('显式标已读失败后沿用请求编号，成功后重读且只发送快照引用', async () => {
  let marked = false;
  const reader = vi.fn(async () =>
    changesFixture(Date.now(), 21, null, snapshot, marked),
  );
  const uninstall = installSessionReader(reader);
  const writer = vi
    .fn()
    .mockRejectedValueOnce(new ApiError('暂不可用', 503))
    .mockImplementation(async () => {
      marked = true;
      const p = changesFixture(Date.now(), 21, null, snapshot, true);
      return {
        status: 'seen',
        vault_uuid: p.vault_uuid,
        project: p.project,
        through: p.through,
      };
    });
  const unwrite = installChangesWriter(writer);
  try {
    const { result } = renderHook(() => useChanges(null));
    await waitFor(() => expect(result.current.page).not.toBeNull());
    expect(writer).not.toHaveBeenCalled();
    await act(async () => result.current.mark());
    expect(result.current.error).toBe('暂不可用');
    await act(async () => result.current.mark());
    expect(writer.mock.calls[0][0]).toEqual(writer.mock.calls[1][0]);
    expect(Object.keys(writer.mock.calls[0][0])).toEqual([
      'vault_uuid',
      'project',
      'snapshot',
      'request_id',
    ]);
    expect(result.current.page?.entries.every((r) => r.unread === false)).toBe(
      true,
    );
    expect(reader).toHaveBeenCalledTimes(2);
  } finally {
    unwrite();
    uninstall();
  }
});
it('隐藏面板不读，过期页清空结果并要求重读第一页', async () => {
  const reader = vi.fn(async (url: string) => {
    if (new URL(url, location.origin).searchParams.has('before'))
      throw new ApiError('快照过期', 409);
    return {
      ...changesFixture(Date.now(), 21, null, snapshot, false),
      next_cursor: snapshot,
    };
  });
  const uninstall = installSessionReader(reader);
  try {
    const { result, rerender } = renderHook(
      ({ active }) => useChanges(null, active),
      { initialProps: { active: false } },
    );
    expect(reader).not.toHaveBeenCalled();
    rerender({ active: true });
    await waitFor(() => expect(result.current.page).not.toBeNull());
    await act(async () => result.current.next());
    expect(result.current).toMatchObject({ page: null, expired: true });
    await act(async () => result.current.reload());
    expect(result.current.page).not.toBeNull();
  } finally {
    uninstall();
  }
});

it('安静重读更新第一页，失败保留已显示事实，晚到响应不能盖过手动重读', async () => {
  const late = deferred<unknown>();
  const base = changesFixture(Date.now(), 21, null, snapshot, false);
  let mode: 'initial' | 'late' | 'new' | 'failed' = 'initial';
  const uninstall = installSessionReader(async () => {
    if (mode === 'late') return late.promise;
    if (mode === 'failed') throw new Error('暂不可用');
    return {
      ...base,
      through: { ...base.through, sequence: mode === 'new' ? '22' : '21' },
    };
  });
  try {
    const { result } = renderHook(() => useChanges(null));
    await waitFor(() => expect(result.current.page).not.toBeNull());
    mode = 'late';
    let pending!: ReturnType<typeof result.current.revalidate>;
    act(() => {
      pending = result.current.revalidate(new AbortController().signal);
    });
    mode = 'new';
    await act(async () => result.current.reload());
    await act(async () => {
      late.resolve(base);
      await pending;
    });
    expect(result.current.page?.through.sequence).toBe('22');
    mode = 'failed';
    await act(async () => {
      await result.current.revalidate(new AbortController().signal);
    });
    expect(result.current.page?.through.sequence).toBe('22');
  } finally {
    uninstall();
  }
});
