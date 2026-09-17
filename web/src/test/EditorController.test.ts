import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { EditorController } from '../EditorController';
import { DraftStore } from '../draftStore';
import { installCanonicalWriter } from '../canonicalApi';
import { ApiError } from '../sessionHttp';
import type { Replacement } from '../canonicalModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';

let disk = { ...canonical };
let editor: EditorController;
let store: DraftStore;
let uninstall: () => void;
const writer =
  vi.fn<(request: Replacement, signal: AbortSignal) => Promise<unknown>>();
beforeEach(async () => {
  vi.useFakeTimers();
  disk = { ...canonical };
  writer.mockReset().mockImplementation(async (request) => {
    disk = { ...disk, content: request.content, revision: 'b'.repeat(64) };
    return committed(request);
  });
  uninstall = installCanonicalWriter(writer);
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => jsonResponse(disk)),
  );
  store = new DraftStore().forScope(ownerScope);
  editor = new EditorController(canonical.path, store);
  await editor.load();
});
afterEach(() => {
  editor.stop();
  uninstall();
  vi.useRealTimers();
});

it('停止输入 1.5 秒保存；回执成为新基线并另行核对磁盘', async () => {
  editor.change('新草稿');
  await vi.advanceTimersByTimeAsync(1499);
  expect(writer).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(writer).toHaveBeenCalledTimes(1);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'clean',
    draft: '新草稿',
    base: { revision: 'b'.repeat(64) },
  });
  expect(store.getSnapshot()).toBe(0);
});
it('IME 组合期间暂停计时与失焦保存，组合结束后再保存', async () => {
  editor.composition(true);
  editor.change('尚未完成的拼音');
  editor.blur();
  await vi.advanceTimersByTimeAsync(5000);
  expect(writer).not.toHaveBeenCalled();
  editor.composition(false);
  await vi.advanceTimersByTimeAsync(1500);
  expect(writer).toHaveBeenCalledTimes(1);
});
it('保存期间继续输入不被响应覆盖；下一次使用回执 revision', async () => {
  const response = deferred<unknown>();
  writer.mockImplementationOnce(() => response.promise);
  editor.change('已发送');
  const saving = editor.save();
  editor.change('已发送，继续输入');
  disk = { ...disk, content: '已发送', revision: 'b'.repeat(64) };
  response.resolve(committed(writer.mock.calls[0][0]));
  await saving;
  expect(editor.getSnapshot()).toMatchObject({
    draft: '已发送，继续输入',
    phase: 'dirty',
    base: { content: '已发送' },
  });
  await vi.advanceTimersByTimeAsync(1500);
  expect(writer.mock.calls[1][0]).toMatchObject({
    expected_revision: 'b'.repeat(64),
    content: '已发送，继续输入',
  });
});
it('CAS 409 保留双方并独立读取现版本，明确继续后才保存', async () => {
  disk = { ...disk, content: '其他端内容', revision: 'c'.repeat(64) };
  writer.mockImplementationOnce(async (request) => {
    throw new ApiError('冲突', 409, {
      code: 'revision_conflict',
      vault_uuid: request.vault_uuid,
      path: request.path,
      expected_revision: request.expected_revision,
      actual_revision: disk.revision,
    });
  });
  editor.change('本地修改');
  await editor.save();
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'conflict',
    draft: '本地修改',
    base: { content: canonical.content },
    remote: { content: '其他端内容' },
  });
  await vi.advanceTimersByTimeAsync(5000);
  expect(writer).toHaveBeenCalledTimes(1);
  editor.continueFromCurrent();
  await editor.save();
  expect(writer.mock.calls[1][0].expected_revision).toBe('c'.repeat(64));
});
it('结果不确定只能恢复原键与原正文，新增输入仍保留', async () => {
  writer.mockRejectedValueOnce(new TypeError('网络中断'));
  editor.change('可能已保存');
  await editor.save();
  const original = writer.mock.calls[0][0];
  editor.change('后续草稿');
  await vi.advanceTimersByTimeAsync(5000);
  await editor.save();
  expect(writer).toHaveBeenCalledTimes(1);
  await editor.save(true);
  expect(writer.mock.calls[1][0]).toEqual(original);
  expect(editor.getSnapshot().draft).toBe('后续草稿');
});
it('陈旧拒绝不自动重试，核验前仍保留草稿', async () => {
  writer.mockRejectedValueOnce(new ApiError('陈旧', 503, { code: 'stale' }));
  editor.change('待保存');
  await editor.save();
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'blocked',
    pending: null,
    draft: '待保存',
  });
  await vi.advanceTimersByTimeAsync(5000);
  expect(writer).toHaveBeenCalledTimes(1);
  await editor.inspect();
  editor.continueFromCurrent();
  await editor.save();
  expect(writer).toHaveBeenCalledTimes(2);
});
it('恢复未知请求再次被陈旧门禁拒绝，不能丢弃原键并假装原请求未提交', async () => {
  writer
    .mockRejectedValueOnce(new TypeError('响应丢失'))
    .mockRejectedValueOnce(new ApiError('尚未追平', 503, { code: 'stale' }));
  editor.change('原提交');
  await editor.save();
  const original = writer.mock.calls[0][0];
  await editor.save(true);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'unknown',
    pending: original,
  });
  await editor.inspect();
  editor.continueFromCurrent();
  expect(editor.getSnapshot().phase).toBe('unknown');
  await editor.save(true);
  expect(writer.mock.calls[2][0]).toEqual(original);
});
it('回执后的磁盘再变化必须进入比较，不能把回执冒充当前磁盘', async () => {
  writer.mockImplementationOnce(async (request) => {
    disk = { ...disk, content: '后来又改了', revision: 'd'.repeat(64) };
    return committed(request);
  });
  editor.change('本次正文');
  await editor.save();
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'conflict',
    base: { content: '本次正文', revision: 'b'.repeat(64) },
    remote: { content: '后来又改了' },
  });
});
