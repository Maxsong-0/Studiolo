import { useState } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import { expect, it, vi } from 'vitest';
import { SessionBoundary } from '../../SessionBoundary';
import { CanonicalEditor } from '../../components/CanonicalEditor';
import { useDraftCount } from '../../draftContext';
import { canonical, ownerScope } from '../canonicalFixtures';
import { closeRequest, nativeClose } from '../dialogSurfaceHelpers';
import { jsonResponse } from '../fixtures';
import { mockSessions, pair, sessionResponse } from '../sessionFixtures';

/*
 * T-170：浏览器的原生关闭请求（Chrome CloseWatcher：Android 返回手势、requestClose()、Esc）
 * 与 Esc 走同一处理——有草稿先弹确认框，确认框开着就等于「继续编辑」；
 * 浏览器坚持关闭（不可取消的 cancel）拦不住时，按「保留内存草稿并关闭」收场，编辑器跟着卸载。
 */

function DraftCount() {
  const { count } = useDraftCount();
  return <output aria-label="内存草稿篇数">{count}</output>;
}

/** 与 App 一样：onClose 回调后把编辑器卸载。 */
function Harness({ onClose }: { onClose: () => void }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <DraftCount />
      {open ? (
        <CanonicalEditor
          path={canonical.path}
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        />
      ) : null}
    </>
  );
}

async function openEditor() {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (String(input).startsWith('/api/v1/file?'))
      return jsonResponse(canonical);
    return sessionResponse(input, init);
  });
  const onClose = vi.fn();
  render(
    <SessionBoundary>{() => <Harness onClose={onClose} />}</SessionBoundary>,
  );
  const source = await screen.findByRole(
    'textbox',
    { name: 'Markdown 源码' },
    { timeout: 5000 },
  );
  act(() => source.focus());
  return {
    source,
    onClose,
    // 保存请求只有这一种：会话链上的 POST /api/v1/mutations。
    saves: () =>
      fetchMock.mock.calls.filter(([path]) => path === '/api/v1/mutations'),
  };
}

function typeDraft(source: HTMLElement, text: string) {
  const view = EditorView.findFromDOM(source)!;
  act(() => {
    view.dispatch({ changes: { from: view.state.doc.length, insert: text } });
  });
}

function editor() {
  return screen.queryByRole('dialog', { name: '编辑与预览' });
}

function leaveDialog() {
  return screen.queryByRole('alertdialog', { name: '处理未保存草稿' });
}

function draftCount() {
  return screen.getByLabelText('内存草稿篇数');
}

/** 在焦点元素上按 Esc；返回 keydown 是否被拦下（拦下了浏览器就不再发关闭请求）。 */
function escapeHandled(init: KeyboardEventInit = {}) {
  const target = document.activeElement ?? document.body;
  return !fireEvent.keyDown(target, { key: 'Escape', code: 'Escape', ...init });
}

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it('可取消的关闭请求（返回手势、requestClose）：有草稿先弹确认框，编辑器不关', async () => {
  const { source, onClose, saves } = await openEditor();
  typeDraft(source, '返回手势时的草稿');
  closeRequest(editor()!, true);
  const leave = leaveDialog()!;
  expect(within(leave).getByRole('button', { name: '继续编辑' })).toHaveFocus();
  expect(editor()).toHaveAttribute('open');
  expect(onClose).not.toHaveBeenCalled();
  await settle();
  expect(saves()).toHaveLength(0);
});

it('确认框开着时再来一次可取消的关闭请求：等于「继续编辑」，焦点回到编辑区', async () => {
  const { source, onClose } = await openEditor();
  typeDraft(source, '连着两次返回手势的草稿');
  closeRequest(editor()!, true);
  expect(leaveDialog()).toBeInTheDocument();
  closeRequest(editor()!, true);
  expect(leaveDialog()).not.toBeInTheDocument();
  expect(source).toHaveFocus();
  expect(editor()).toHaveAttribute('open');
  expect(onClose).not.toHaveBeenCalled();
});

it('没有草稿时可取消的关闭请求直接关闭编辑器', async () => {
  const { onClose } = await openEditor();
  closeRequest(editor()!, true);
  expect(onClose).toHaveBeenCalledOnce();
  expect(editor()).not.toBeInTheDocument();
});

it('连按 Esc：每一下都在 keydown 拦下，只在确认框与继续编辑之间切换，编辑器不会被浏览器强制关闭', async () => {
  const { source, onClose, saves } = await openEditor();
  typeDraft(source, '连按 Esc 的草稿');
  for (let round = 0; round < 3; round += 1) {
    expect(escapeHandled()).toBe(true);
    expect(leaveDialog()).toBeInTheDocument();
    expect(escapeHandled()).toBe(true);
    expect(leaveDialog()).not.toBeInTheDocument();
    expect(source).toHaveFocus();
  }
  expect(editor()).toHaveAttribute('open');
  expect(onClose).not.toHaveBeenCalled();
  await settle();
  expect(saves()).toHaveLength(0);
});

it('编辑区里已被用掉的 Esc、输入法组字中的 Esc 都不当关闭请求', async () => {
  const { source } = await openEditor();
  typeDraft(source, '收起补全用的 Esc');
  // 模拟编辑器自己用掉这一下（如收起自动补全）：它在编辑区上先拦下。
  source.addEventListener('keydown', (event) => event.preventDefault(), {
    once: true,
  });
  escapeHandled();
  expect(leaveDialog()).not.toBeInTheDocument();
  expect(escapeHandled({ isComposing: true })).toBe(false);
  expect(leaveDialog()).not.toBeInTheDocument();
  expect(editor()).toHaveAttribute('open');
});

it('浏览器强制关闭（不可取消的 cancel）：按「保留内存草稿并关闭」收场，不弹框、不发保存、只回调一次', async () => {
  const { source, onClose, saves } = await openEditor();
  typeDraft(source, '被强制关闭时的草稿');
  const dialog = editor()!;
  // cancel 与 close 之间：拦不住的请求不改页面状态，不先弹出确认框。
  fireEvent(dialog, new Event('cancel', { cancelable: false }));
  expect(leaveDialog()).not.toBeInTheDocument();
  nativeClose(dialog);
  expect(onClose).toHaveBeenCalledOnce();
  expect(editor()).not.toBeInTheDocument();
  expect(draftCount()).toHaveTextContent('1');
  await settle();
  expect(saves()).toHaveLength(0);
});

it('确认框开着时被浏览器强制关闭：同样保留草稿并关闭，只回调一次', async () => {
  const { source, onClose, saves } = await openEditor();
  typeDraft(source, '框开着时被强制关闭的草稿');
  expect(escapeHandled()).toBe(true);
  expect(leaveDialog()).toBeInTheDocument();
  const dialog = editor()!;
  // 拦不住的请求也不把框收起（那等于「继续编辑」，而编辑器马上就要关了）。
  fireEvent(dialog, new Event('cancel', { cancelable: false }));
  expect(leaveDialog()).toBeInTheDocument();
  nativeClose(dialog);
  expect(onClose).toHaveBeenCalledOnce();
  expect(editor()).not.toBeInTheDocument();
  expect(draftCount()).toHaveTextContent('1');
  await settle();
  expect(saves()).toHaveLength(0);
});
