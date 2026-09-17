import type { ReactNode } from 'react';
import { act, renderHook } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useEditorNavigation } from '../useEditorNavigation';
import { FileOperationContext } from '../fileOperationContext';
import { FileOperationStore } from '../fileOperationStore';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming, renameResponse } from './renameFixtures';
import { fixture } from './fixtures';
import { canonical } from './canonicalFixtures';
import { deferred } from './sessionFixtures';
import type { Workbench } from '../types';

const fresh = {
  ...fixture,
  freshness: {
    ...canonical.freshness!,
    indexed_through: '7',
    committed_through: '7',
  },
};
async function setup(refresh: () => Promise<Workbench | null>) {
  const root = new FileOperationStore();
  let scoped = root.forScope('owner');
  const uninstall = installFileOperationWriter(async () => renameResponse());
  await scoped.submit(renaming, new AbortController().signal);
  uninstall();
  const receipt = scoped.getSnapshot()!.receipt!;
  const wrapper = ({ children }: { children: ReactNode }) => (
    <FileOperationContext.Provider value={scoped}>
      {children}
    </FileOperationContext.Provider>
  );
  const hook = renderHook(() => useEditorNavigation(refresh), { wrapper });
  act(() => hook.result.current.open(renaming.from));
  return {
    ...hook,
    root,
    receipt,
    changeScope: () => {
      scoped = root.forScope('other');
      hook.rerender();
    },
  };
}
it('fresh双水位后新实例打开目标，Vault绑定保留且不沿用源路径revision', async () => {
  const refresh = vi.fn().mockResolvedValue(fresh);
  const view = await setup(refresh);
  const previous = view.result.current.current;
  await act(async () => {
    expect(
      await view.result.current.onRenamed(
        renaming,
        view.receipt,
        new AbortController().signal,
        () => true,
      ),
    ).toBe(true);
  });
  expect(view.result.current.current).toMatchObject({
    path: renaming.to,
    vault: renaming.vault_uuid,
  });
  expect(view.result.current.current?.id).not.toBe(previous?.id);
  expect(refresh).toHaveBeenCalledOnce();
});
it.each(['stale', 'behind', 'failure'] as const)(
  '%s不挪动原editor，保留回执供明确再次读取',
  async (mode) => {
    const snapshot =
      mode === 'failure'
        ? null
        : {
            ...fresh,
            freshness: {
              ...fresh.freshness,
              ...(mode === 'stale'
                ? { stale: true, reason: 'changed' }
                : { indexed_through: '6', committed_through: '6' }),
            },
          };
    const refresh = vi.fn().mockResolvedValue(snapshot);
    const view = await setup(refresh);
    await act(async () => {
      expect(
        await view.result.current.onRenamed(
          renaming,
          view.receipt,
          new AbortController().signal,
          () => true,
        ),
      ).toBe(false);
    });
    expect(view.result.current.current?.path).toBe(renaming.from);
    expect(view.root.forScope('owner').getSnapshot()?.receipt).toEqual(
      view.receipt,
    );
    expect(refresh).toHaveBeenCalledOnce();
  },
);
it.each([
  'close',
  'reopen',
  'other',
  'scope',
  'abort',
  'editing',
  'dismiss',
] as const)('等待中%s阻止迟到回调挪走或重新打开对话框', async (mode) => {
  const delayed = deferred<Workbench | null>();
  const view = await setup(() => delayed.promise);
  const signal = new AbortController();
  let current = true;
  let opening!: Promise<boolean>;
  act(() => {
    opening = view.result.current.onRenamed(
      renaming,
      view.receipt,
      signal.signal,
      () => current,
    );
  });
  act(() => {
    if (mode === 'close') view.result.current.open(null);
    if (mode === 'reopen') {
      view.result.current.open(null);
      view.result.current.open(renaming.from);
    }
    if (mode === 'other') view.result.current.open('笔记/另一篇.md');
    if (mode === 'scope') view.changeScope();
    if (mode === 'abort') signal.abort();
    if (mode === 'editing') current = false;
    if (mode === 'dismiss') view.root.forScope('owner').dismiss();
  });
  const selected = view.result.current.current;
  await act(async () => {
    delayed.resolve(fresh);
    expect(await opening).toBe(false);
  });
  expect(view.result.current.current).toEqual(selected);
});
it('与原Store不同的请求/回执不触发刷新，即使字段看似合法', async () => {
  const refresh = vi.fn().mockResolvedValue(fresh);
  const view = await setup(refresh);
  await act(async () => {
    expect(
      await view.result.current.onRenamed(
        { ...renaming, to: '笔记/第三处.md' },
        view.receipt,
        new AbortController().signal,
        () => true,
      ),
    ).toBe(false);
    expect(
      await view.result.current.onRenamed(
        renaming,
        { ...view.receipt },
        new AbortController().signal,
        () => true,
      ),
    ).toBe(false);
  });
  expect(refresh).not.toHaveBeenCalled();
});
