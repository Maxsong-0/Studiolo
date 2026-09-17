import { createElement } from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, vi } from 'vitest';
import { EditorController } from '../EditorController';
import { DraftContext } from '../draftContext';
import { DraftStore } from '../draftStore';
import { FileOperationContext } from '../fileOperationContext';
import { FileOperationStore } from '../fileOperationStore';
import type { Replacement } from '../canonicalModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';
import { RenameFileFixture } from './RenameFileFixture';

export async function setup(enabled = true) {
  let disk = { ...canonical };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) =>
      String(input) === '/api/v1/file-operations'
        ? jsonResponse({
            enabled: true,
            vault_uuid: canonical.vault_uuid,
            ...(enabled ? { rename_enabled: true } : {}),
          })
        : jsonResponse(disk),
    ),
  );
  const drafts = new DraftStore().forScope(ownerScope);
  const files = new FileOperationStore().forScope(ownerScope);
  const controller = new EditorController(
    canonical.path,
    drafts,
    () => !files.hasPending(),
  );
  await controller.load();
  const view = render(
    createElement(
      DraftContext.Provider,
      { value: drafts },
      createElement(
        FileOperationContext.Provider,
        { value: files },
        createElement(RenameFileFixture, { controller }),
      ),
    ),
  );
  return {
    ...view,
    drafts,
    files,
    controller,
    disk: async (request: Replacement) => {
      disk = { ...disk, content: request.content, revision: 'e'.repeat(64) };
      return committed(request, disk.revision);
    },
  };
}
export async function review(to = '笔记/新标题.md') {
  const toggle = await screen.findByRole('button', { name: '重命名文件' });
  await waitFor(() => expect(toggle).toBeEnabled());
  fireEvent.click(toggle);
  fireEvent.change(screen.getByRole('textbox', { name: '目标完整路径' }), {
    target: { value: to },
  });
  fireEvent.click(screen.getByRole('checkbox'));
}
