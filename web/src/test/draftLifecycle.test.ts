import { afterEach, expect, it, vi } from 'vitest';
import { DraftStore } from '../draftStore';
import { EditorController } from '../EditorController';
import { installCanonicalWriter } from '../canonicalApi';
import { canonical, ownerScope, replacement } from './canonicalFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';

afterEach(() => vi.useRealTimers());
it('锁定不展示、同主人可恢复，换主人/无scope/另一Vault均不能读取旧稿', () => {
  const root = new DraftStore();
  const owner = root.forScope(ownerScope);
  const record = { base: canonical, draft: '主人草稿', pending: replacement };
  expect(owner.keep(record)).toBe(true);
  expect(root.list()).toEqual([]);
  expect(root.forScope(null).get(canonical)).toBeUndefined();
  expect(root.forScope('e'.repeat(64)).list()).toEqual([]);
  expect(
    owner.get({
      ...canonical,
      vault_uuid: '87654321-1234-1234-1234-123456789abc',
    }),
  ).toBeUndefined();
  expect(root.forScope(ownerScope).get(canonical)).toEqual(record);
  root.destroy();
  expect(owner.getSnapshot()).toBe(0);
  expect(owner.keep(record)).toBe(false);
});
it('满仓不逐出已有稿；总正文预算包括未决提交', () => {
  const store = new DraftStore().forScope(ownerScope);
  for (let index = 0; index < 8; index += 1)
    expect(
      store.keep({
        base: { ...canonical, path: `${index}.md` },
        draft: '保留',
        pending: null,
      }),
    ).toBe(true);
  expect(store.keep({ base: canonical, draft: '第九篇', pending: null })).toBe(
    false,
  );
  expect(store.getSnapshot()).toBe(8);
  expect(store.list().every((record) => record.draft === '保留')).toBe(true);
  const bounded = new DraftStore().forScope(ownerScope);
  expect(
    bounded.keep({
      base: canonical,
      draft: '中'.repeat(1500000),
      pending: replacement,
    }),
  ).toBe(false);
  expect(bounded.getSnapshot()).toBe(0);
});
it('重新挂载先读当前磁盘，人工恢复仍不自动POST；换文档不复用旧稿', async () => {
  vi.useFakeTimers();
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => jsonResponse(canonical)),
  );
  const writer = vi.fn();
  const uninstall = installCanonicalWriter(writer);
  const store = new DraftStore().forScope(ownerScope);
  const first = new EditorController(canonical.path, store);
  await first.load();
  first.change('未保存输入');
  first.stop();
  const second = new EditorController(canonical.path, store);
  await second.load();
  expect(second.getSnapshot()).toMatchObject({
    phase: 'restore',
    draft: canonical.content,
  });
  second.restore();
  expect(second.getSnapshot()).toMatchObject({
    phase: 'review',
    draft: '未保存输入',
    remote: { content: canonical.content },
  });
  await vi.advanceTimersByTimeAsync(5000);
  expect(writer).not.toHaveBeenCalled();
  expect(store.get({ ...canonical, path: '另一页.md' })).toBeUndefined();
  second.stop();
  uninstall();
});
it('取消等待保留不可变原请求；迟到响应不覆盖已锁定的稿', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => jsonResponse(canonical)),
  );
  const response = deferred<unknown>();
  const uninstall = installCanonicalWriter(() => response.promise);
  const store = new DraftStore().forScope(ownerScope);
  const editor = new EditorController(canonical.path, store);
  await editor.load();
  editor.change('已发正文');
  const saving = editor.save();
  editor.change('后续未发正文');
  editor.stop();
  const before = store.get(canonical)!;
  expect(before.pending?.content).toBe('已发正文');
  expect(before.draft).toBe('后续未发正文');
  response.resolve({
    status: 'committed',
    receipt: {
      path: canonical.path,
      previous_revision: canonical.revision,
      new_revision: 'b'.repeat(64),
    },
  });
  await saving;
  expect(store.get(canonical)).toEqual(before);
  uninstall();
});
