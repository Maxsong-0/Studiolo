import { expect, it, vi } from 'vitest';
import { FileOperationStore } from '../fileOperationStore';
import { installFileOperationWriter } from '../fileOperationApi';
import { ApiError } from '../sessionHttp';
import { deferred } from './sessionFixtures';
import { renaming, renameResponse } from './renameFixtures';
import { deleting } from './fileOperationFixtures';

it('双击、未知同键和变更目标不重派；核对后保存原双路径回执', async () => {
  const first = deferred<unknown>();
  const writer = vi
    .fn()
    .mockReturnValueOnce(first.promise)
    .mockResolvedValue(renameResponse());
  const uninstall = installFileOperationWriter(writer);
  const root = new FileOperationStore();
  const store = root.forScope('owner');
  const signal = new AbortController().signal;
  try {
    const pending = store.submit(renaming, signal);
    await store.submit(renaming, signal);
    first.reject(new TypeError('response lost'));
    await pending;
    expect(store.getSnapshot()).toMatchObject({
      phase: 'uncertain',
      request: renaming,
    });
    expect(store.getSnapshot()?.message).toContain('目标可能已创建');
    store.dismiss();
    await store.submit({ ...renaming, to: '笔记/另一目标.md' }, signal);
    await store.submit({ ...renaming, idempotency_key: 'new-key' }, signal);
    await root.forScope('other').submit(deleting, signal);
    expect(writer).toHaveBeenCalledTimes(1);
    await store.submit(renaming, signal);
    expect(writer.mock.calls[0][0]).toEqual(writer.mock.calls[1][0]);
    expect(writer.mock.calls[0][0]).not.toHaveProperty('path');
    expect(store.getSnapshot()?.receipt?.effects).toHaveLength(2);
  } finally {
    uninstall();
  }
});
it('abort只结束视图传输，不把已发 Rename 说成取消；replay的403保持未知', async () => {
  const pending = deferred<unknown>();
  const writer = vi
    .fn()
    .mockReturnValueOnce(pending.promise)
    .mockRejectedValue(new ApiError('changed policy', 403));
  const uninstall = installFileOperationWriter(writer);
  const root = new FileOperationStore();
  const store = root.forScope('a');
  const life = new AbortController();
  try {
    const sending = store.submit(renaming, life.signal);
    life.abort();
    pending.reject(new DOMException('aborted', 'AbortError'));
    await sending;
    expect(store.getSnapshot()?.phase).toBe('uncertain');
    expect(root.forScope('b').getSnapshot()).toBeNull();
    await store.submit(renaming, new AbortController().signal);
    expect(store.getSnapshot()?.phase).toBe('uncertain');
    expect(writer.mock.calls[1][0]).toEqual(renaming);
  } finally {
    uninstall();
  }
});
it('200坏双效果仍未知，不能通过旧单效果验证结束原键', async () => {
  const response = renameResponse();
  response.receipt.effects.pop();
  const writer = vi.fn().mockResolvedValue(response);
  const uninstall = installFileOperationWriter(writer);
  const store = new FileOperationStore().forScope('owner');
  try {
    await store.submit(renaming, new AbortController().signal);
    expect(store.getSnapshot()?.phase).toBe('uncertain');
    expect(store.getSnapshot()?.receipt).toBeUndefined();
  } finally {
    uninstall();
  }
});
