import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { EditorController } from '../EditorController';
import {
  installCanonicalRestorer,
  installCanonicalWriter,
} from '../canonicalApi';
import { DraftStore } from '../draftStore';
import { ApiError } from '../sessionHttp';
import type { Restoration } from '../canonicalModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { historyFile } from './historyFixtures';
import { deferred } from './sessionFixtures';
import { jsonResponse } from './fixtures';

let disk = canonical;
let editor: EditorController;
let store: DraftStore;
let uninstall: () => void;
let uninstallWriter: () => void;
const restore =
  vi.fn<(request: Restoration, signal: AbortSignal) => Promise<unknown>>();
const writer = vi.fn();
beforeEach(async () => {
  vi.useFakeTimers();
  disk = { ...canonical };
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => jsonResponse(disk)),
  );
  restore.mockReset().mockImplementation(async (request) => {
    disk = {
      ...disk,
      revision: request.source_revision,
      content: historyFile.content,
    };
    return committed(request, request.source_revision);
  });
  writer.mockReset();
  uninstall = installCanonicalRestorer(restore);
  uninstallWriter = installCanonicalWriter(writer);
  store = new DraftStore().forScope(ownerScope);
  editor = new EditorController(canonical.path, store);
  await editor.load();
});
afterEach(() => {
  editor.stop();
  uninstall();
  uninstallWriter();
  vi.useRealTimers();
});

it('干净页面显式恢复；回执之后另读磁盘，更新同一个编辑状态', async () => {
  await editor.restoreHistory(historyFile);
  expect(restore.mock.calls[0][0]).toMatchObject({
    kind: 'restore',
    source_revision: historyFile.revision,
    expected_revision: canonical.revision,
  });
  expect(restore.mock.calls[0][0]).not.toHaveProperty('content');
  expect(writer).not.toHaveBeenCalled();
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'clean',
    draft: historyFile.content,
    base: { revision: historyFile.revision },
  });
  expect(store.getSnapshot()).toBe(0);
});
it('未处理草稿、在途保存和跨 Vault/path 版本不能被恢复覆盖', async () => {
  await editor.restoreHistory({ ...historyFile, path: '另一页.md' });
  await editor.restoreHistory({ ...historyFile, vault_uuid: '另一库' });
  editor.change('本地未提交');
  await editor.restoreHistory(historyFile);
  expect(editor.getSnapshot().draft).toBe('本地未提交');
  const response = deferred<unknown>();
  writer.mockReturnValueOnce(response.promise);
  const saving = editor.save();
  await editor.restoreHistory(historyFile);
  expect(restore).not.toHaveBeenCalled();
  response.reject(new ApiError('拒绝', 403));
  await saving;
});
it('恢复在途遇到较晚输入，也不能用恢复回执覆盖输入', async () => {
  const response = deferred<unknown>();
  restore.mockReturnValueOnce(response.promise);
  const saving = editor.restoreHistory(historyFile);
  editor.change('晚到的输入');
  disk = { ...canonical, ...historyFile };
  response.resolve(committed(restore.mock.calls[0][0], historyFile.revision));
  await saving;
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'dirty',
    draft: '晚到的输入',
    base: { content: historyFile.content },
  });
});
it('审计明确未安装时清本次 pending 并阻断，不会自动换普通保存重试', async () => {
  restore.mockRejectedValueOnce(
    new ApiError('审计不可用', 503, { code: 'audit_unavailable' }),
  );
  await editor.restoreHistory(historyFile);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'blocked',
    pending: null,
    draft: canonical.content,
  });
  expect(editor.getSnapshot().message).toContain('未安装');
  await vi.advanceTimersByTimeAsync(5000);
  expect(restore).toHaveBeenCalledOnce();
  expect(writer).not.toHaveBeenCalled();
});
it('未知恢复请求跨关闭后同主人手动恢复，审计拒绝仍保留最初 key', async () => {
  restore.mockRejectedValueOnce(
    new ApiError('结果未知', 503, { code: 'outcome_unknown' }),
  );
  await editor.restoreHistory(historyFile);
  const request = restore.mock.calls[0][0];
  editor.stop();
  editor = new EditorController(canonical.path, store);
  await editor.load();
  expect(editor.getSnapshot().phase).toBe('restore');
  editor.restore();
  restore.mockRejectedValueOnce(
    new ApiError('审计不可用', 503, { code: 'audit_unavailable' }),
  );
  await editor.save(true);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'unknown',
    pending: { ...request, content: historyFile.content },
  });
  await vi.advanceTimersByTimeAsync(5000);
  expect(restore).toHaveBeenCalledTimes(2);
  await editor.save(true);
  expect(restore.mock.calls.map(([value]) => value)).toEqual([
    request,
    request,
    request,
  ]);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'clean',
    draft: historyFile.content,
  });
  expect(writer).not.toHaveBeenCalled();
});
it('恢复 CAS 冲突保留原内容；成功回执之后磁盘另变也不冒充当前一致', async () => {
  disk = { ...canonical, revision: 'c'.repeat(64), content: '别的端先改了' };
  restore.mockImplementationOnce(async (request) => {
    throw new ApiError('冲突', 409, {
      code: 'revision_conflict',
      vault_uuid: request.vault_uuid,
      path: request.path,
      expected_revision: request.expected_revision,
      actual_revision: disk.revision,
    });
  });
  await editor.restoreHistory(historyFile);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'conflict',
    draft: canonical.content,
    pending: null,
    remote: { content: '别的端先改了' },
  });
  editor.discard();
  restore.mockImplementationOnce(async (request) => {
    disk = { ...disk, revision: 'd'.repeat(64), content: '恢复回执之后又变了' };
    return committed(request, historyFile.revision);
  });
  await editor.restoreHistory(historyFile);
  expect(editor.getSnapshot()).toMatchObject({
    phase: 'conflict',
    draft: historyFile.content,
    base: { revision: historyFile.revision },
    remote: { revision: 'd'.repeat(64) },
  });
});
