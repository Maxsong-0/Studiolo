import { createElement } from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, vi } from 'vitest';
import { DraftStore } from '../draftStore';
import { FileOperationStore } from '../fileOperationStore';
import type { FileOperationRequest } from '../fileOperationModel';
import { CanonicalRenameHarness } from './CanonicalRenameHarness';
import { canonical, ownerScope } from './canonicalFixtures';
import { renaming, renameResponse } from './renameFixtures';
import { fixture, jsonResponse } from './fixtures';

export function setup() {
  const drafts = new DraftStore().forScope(ownerScope);
  const files = new FileOperationStore().forScope(ownerScope);
  const view = render(createElement(CanonicalRenameHarness, { drafts, files }));
  return { ...view, files };
}
export const fresh = {
  ...fixture,
  freshness: {
    ...canonical.freshness!,
    indexed_through: '7',
    committed_through: '7',
  },
};
export function service(
  refresh: () => Promise<Response>,
  targetVault = canonical.vault_uuid,
  targetEditable = false,
) {
  const fetcher = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'https://fixture.invalid');
    if (url.pathname === '/api/workbench')
      return url.search ? refresh() : jsonResponse(fixture);
    if (url.pathname === '/api/v1/file-operations')
      return jsonResponse({
        enabled: true,
        rename_enabled: true,
        vault_uuid: canonical.vault_uuid,
      });
    if (url.pathname === '/api/v1/file') {
      const path = url.searchParams.get('path');
      if (path === canonical.path) return jsonResponse(canonical);
      if (path === renaming.to)
        return jsonResponse({
          ...canonical,
          path,
          vault_uuid: targetVault,
          revision: 'b'.repeat(64),
          content: '独立读取的当前目标',
          editing: {
            allowed: targetEditable,
            reason: targetEditable ? null : 'read_only',
            max_bytes: 262144,
          },
        });
    }
    throw new Error(`fixture unexpected ${url.pathname}`);
  });
  vi.stubGlobal('fetch', fetcher);
  return fetcher;
}
export async function start() {
  fireEvent.click(screen.getByRole('button', { name: '打开源' }));
  await screen.findByRole('textbox', { name: 'Markdown 源码' });
  const rename = await screen.findByRole('button', { name: '重命名文件' });
  await waitFor(() => expect(rename).toBeEnabled());
  fireEvent.click(rename);
  fireEvent.change(screen.getByRole('textbox', { name: '目标完整路径' }), {
    target: { value: renaming.to },
  });
  fireEvent.click(screen.getByRole('checkbox'));
  fireEvent.click(screen.getByRole('button', { name: '确认重命名' }));
}
export const accept = async (request: FileOperationRequest) => {
  if (request.kind !== 'rename') throw new Error('rename only');
  return renameResponse(request);
};
