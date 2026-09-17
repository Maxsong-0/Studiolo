import { expect, it, vi } from 'vitest';
import { FileOperationStore } from '../fileOperationStore';
import { installFileOperationWriter } from '../fileOperationApi';
import { ApiError } from '../sessionHttp';
import {
  deleting,
  fileReceipt,
  restoring,
  tombstone,
} from './fileOperationFixtures';
import { deferred } from './sessionFixtures';

it('未知操作保留原键，双击及不同新键不发送，明确恢复只复用原请求', async () => {
  const first = deferred<unknown>();
  const writer = vi
    .fn()
    .mockReturnValueOnce(first.promise)
    .mockResolvedValue(fileReceipt());
  const uninstall = installFileOperationWriter(writer);
  const store = new FileOperationStore().forScope('owner');
  const signal = new AbortController().signal;
  try {
    const sending = store.submit(deleting, signal);
    await store.submit(deleting, signal);
    first.reject(new Error('connection lost'));
    await sending;
    store.dismiss();
    expect(store.getSnapshot()?.phase).toBe('uncertain');
    await store.submit({ ...deleting, idempotency_key: 'another-key' }, signal);
    expect(writer).toHaveBeenCalledTimes(1);
    await store.submit(deleting, signal);
    expect(writer).toHaveBeenCalledTimes(2);
    expect(writer.mock.calls[0][0]).toEqual(writer.mock.calls[1][0]);
    expect(store.getSnapshot()?.phase).toBe('committed');
    expect(store.hasPending()).toBe(false);
  } finally {
    uninstall();
  }
});
it('原请求复核遇到403仍保持未知，不误判原动作未执行', async () => {
  const writer = vi
    .fn()
    .mockRejectedValueOnce(new Error('network'))
    .mockRejectedValueOnce(new ApiError('policy changed', 403));
  const uninstall = installFileOperationWriter(writer);
  const store = new FileOperationStore().forScope('owner');
  try {
    await store.submit(deleting, new AbortController().signal);
    await store.submit(deleting, new AbortController().signal);
    expect(store.getSnapshot()?.phase).toBe('uncertain');
  } finally {
    uninstall();
  }
});
it('主人隔离保留记录，失去scope不暴露，销毁后迟到响应不会复活', async () => {
  const pending = deferred<unknown>();
  const writer = vi.fn().mockReturnValue(pending.promise);
  const uninstall = installFileOperationWriter(writer);
  const root = new FileOperationStore();
  const a = root.forScope('a');
  try {
    const sending = a.submit(deleting, new AbortController().signal);
    expect(root.forScope('b').getSnapshot()).toBeNull();
    expect(root.forScope(null).getSnapshot()).toBeNull();
    expect(root.forScope('a').getSnapshot()?.request).toEqual(deleting);
    root.destroy();
    pending.resolve(fileReceipt());
    await sending;
    expect(a.getSnapshot()).toBeNull();
    expect(root.hasPending()).toBe(false);
  } finally {
    uninstall();
  }
});
it('未知恢复携带原墓碑验证事实，错误200回执不能确认成功', async () => {
  const response = fileReceipt(restoring);
  response.receipt.effects[0].after = {
    ...response.receipt.effects[0].after,
    bytes: 0,
  };
  const writer = vi.fn().mockResolvedValue(response);
  const uninstall = installFileOperationWriter(writer);
  const store = new FileOperationStore().forScope('owner');
  try {
    await store.submit(restoring, new AbortController().signal, tombstone);
    expect(store.getSnapshot()?.phase).toBe('uncertain');
    expect(store.getSnapshot()?.restoreSource?.sha256).toBe(tombstone.sha256);
    expect(writer.mock.calls[0][0]).not.toHaveProperty('restoreSource');
  } finally {
    uninstall();
  }
});
it('首次明确拒绝可以结束，未装配写者不会绕过会话直接fetch', async () => {
  const fetch = vi.fn();
  vi.stubGlobal('fetch', fetch);
  const store = new FileOperationStore().forScope('owner');
  await store.submit(deleting, new AbortController().signal);
  expect(store.getSnapshot()?.phase).toBe('rejected');
  expect(fetch).not.toHaveBeenCalled();
  store.dismiss();
  expect(store.getSnapshot()).toBeNull();
});
