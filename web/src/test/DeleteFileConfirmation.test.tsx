import { useSyncExternalStore } from 'react';
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { DeleteFile } from '../components/DeleteFile';
import { EditorController } from '../EditorController';
import { DraftStore } from '../draftStore';
import {
  FileOperationBoundary,
  FileOperationScopeBoundary,
} from '../FileOperationBoundary';
import { installCanonicalWriter } from '../canonicalApi';
import { installFileOperationWriter } from '../fileOperationApi';
import type { Replacement } from '../canonicalModel';
import type { FileOperationRequest } from '../fileOperationModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { fileReceipt } from './fileOperationFixtures';
import { jsonResponse } from './fixtures';

// 仅用轻量输入表面驱动真实控制器；自动保存、草稿与删除 Store 都不替换。
function Editing({ editor }: { editor: EditorController }) {
  const state = useSyncExternalStore(editor.subscribe, editor.getSnapshot);
  return (
    <>
      <textarea
        aria-label="测试正文"
        value={state.draft}
        onChange={(event) => editor.change(event.target.value)}
      />
      <DeleteFile controller={editor} state={state} />
    </>
  );
}

it('删除确认不能跨越编辑和自动保存；重新审阅后只提交精确 R2', async () => {
  const revision2 = 'b'.repeat(64);
  const body2 = `${canonical.content}自动保存后的第二版\n`;
  let disk = { ...canonical };
  const fetcher = vi.fn(async (input: RequestInfo | URL) => {
    if (String(input) === '/api/v1/file-operations') {
      return jsonResponse({ enabled: true, vault_uuid: canonical.vault_uuid });
    }
    if (String(input).startsWith('/api/v1/file?')) return jsonResponse(disk);
    throw new Error(`未预期的读取：${String(input)}`);
  });
  vi.stubGlobal('fetch', fetcher);
  const save = vi.fn(async (request: Replacement) => {
    disk = { ...disk, content: request.content, revision: revision2 };
    return committed(request, revision2);
  });
  const deletion = vi.fn(async (request: FileOperationRequest) => {
    if (request.kind !== 'delete') throw new Error('只允许本项删除夹具');
    const response = fileReceipt(request);
    response.receipt.effects[0].before = {
      ...response.receipt.effects[0].before,
      revision: request.expected_revision,
    };
    return response;
  });
  const uninstallSave = installCanonicalWriter(save);
  const uninstallDelete = installFileOperationWriter(deletion);
  const drafts = new DraftStore().forScope(ownerScope);
  const editor = new EditorController(canonical.path, drafts);
  await editor.load();
  const view = render(
    <FileOperationBoundary>
      <FileOperationScopeBoundary scope={ownerScope}>
        <Editing editor={editor} />
      </FileOperationScopeBoundary>
    </FileOperationBoundary>,
  );
  try {
    const toggle = await screen.findByRole('button', { name: '删除文件' });
    // 能力读取完成后才能开始审阅，不能把 disabled 按钮当作没有发请求的证据。
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);
    const checkbox = screen.getByRole('checkbox', {
      name: '我确认删除这个已保存版本',
    });
    fireEvent.click(checkbox);
    expect(checkbox).toBeChecked();
    expect(screen.getByRole('button', { name: '确认删除' })).toBeEnabled();

    vi.useFakeTimers();
    fireEvent.change(screen.getByRole('textbox', { name: '测试正文' }), {
      target: { value: body2 },
    });
    expect(editor.getSnapshot().phase).toBe('dirty');
    expect(drafts.get(canonical)?.draft).toBe(body2);
    expect(checkbox).not.toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }));
    expect(deletion).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(save).toHaveBeenCalledOnce();
    expect(save.mock.calls[0][0]).toMatchObject({
      content: body2,
      expected_revision: canonical.revision,
    });
    expect(editor.getSnapshot()).toMatchObject({
      phase: 'clean',
      draft: body2,
      base: { revision: revision2 },
    });
    expect(drafts.getSnapshot()).toBe(0);
    expect(checkbox).not.toBeChecked();
    expect(screen.getByRole('button', { name: '确认删除' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }));
    expect(deletion).not.toHaveBeenCalled();

    // 关闭旧审阅再展开，必须重新勾选，不借原来 R1 的确认。
    fireEvent.click(toggle);
    fireEvent.click(toggle);
    const renewed = screen.getByRole('checkbox', {
      name: '我确认删除这个已保存版本',
    });
    expect(renewed).toBeEnabled();
    expect(renewed).not.toBeChecked();
    fireEvent.click(renewed);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '确认删除' }));
    });
    expect(deletion).toHaveBeenCalledOnce();
    expect(deletion.mock.calls[0][0]).toEqual({
      kind: 'delete',
      vault_uuid: canonical.vault_uuid,
      path: canonical.path,
      expected_revision: revision2,
      idempotency_key: expect.any(String),
    });
    expect(editor.getSnapshot().phase).toBe('file-operation');
  } finally {
    view.unmount();
    editor.stop();
    uninstallSave();
    uninstallDelete();
    vi.useRealTimers();
  }
});
