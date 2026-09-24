import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { EditorView } from '@codemirror/view';
import { expect, it, vi } from 'vitest';
import { SessionBoundary } from '../../SessionBoundary';
import { CanonicalEditor } from '../../components/CanonicalEditor';
import { useDraftCount } from '../../draftContext';
import { exportDrafts } from '../../draftExport';
import { canonical, ownerScope } from '../canonicalFixtures';
import { jsonResponse } from '../fixtures';
import { mockSessions, pair, sessionResponse } from '../sessionFixtures';

/*
 * T-161：编辑器按 Esc 弹出的「草稿尚未处理」确认框。
 * 失焦自动保存那一路要真实 CodeMirror 才走得到（blur 处理在 MarkdownEditor 里），所以这里不替换输入表面；
 * 停顿计时那一路见 leaveAutosave.test.tsx。
 */

// 下载要走 Blob 与 <a download>，jsdom 不支持，这里只数调用；编辑器、控制器、会话写入都用真实代码。
vi.mock('../../draftExport', () => ({ exportDrafts: vi.fn() }));

function DraftCount() {
  const { count } = useDraftCount();
  return <output aria-label="内存草稿篇数">{count}</output>;
}

/** 打开真实编辑器并把焦点放进正文：之后弹框、点按钮，都要经过编辑区的失焦。 */
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
    <SessionBoundary>
      {() => (
        <>
          <DraftCount />
          <CanonicalEditor path={canonical.path} onClose={onClose} />
        </>
      )}
    </SessionBoundary>,
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

/** 浏览器里的 Esc：先在焦点元素上派发 keydown，没被拦下才向顶层模态框发 cancel（jsdom 不做后一步）。 */
function pressEscape() {
  const target = document.activeElement ?? document.body;
  if (fireEvent.keyDown(target, { key: 'Escape', code: 'Escape' }))
    fireEvent(
      screen.getByRole('dialog', { name: '编辑与预览' }),
      new Event('cancel', { cancelable: true }),
    );
}

function leaveDialog() {
  return screen.queryByRole('alertdialog', { name: '处理未保存草稿' });
}

/** 框里的按钮：编辑器工具行也有一个同名的「导出草稿与恢复信息」，所以限定在框内找。 */
function leaveButton(name: string) {
  const leave = screen.getByRole('alertdialog', { name: '处理未保存草稿' });
  return within(leave).getByRole('button', { name });
}

/** 保存链若在请求名额处排队，会晚一拍才到 fetch；断言「没发」之前先让它走完。 */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it('有草稿时按 Esc：确认框整块是不保存区，焦点落在「继续编辑」，失焦不发保存请求', async () => {
  const { source, saves } = await openEditor();
  typeDraft(source, '尚未保存的草稿');
  expect(screen.getByLabelText('内存草稿篇数')).toHaveTextContent('1');
  pressEscape();
  const leave = screen.getByRole('alertdialog', { name: '处理未保存草稿' });
  expect(leave).toHaveAttribute('data-editor-no-save');
  const resume = within(leave).getByRole('button', { name: '继续编辑' });
  expect(resume).toHaveFocus();
  await settle();
  expect(saves()).toHaveLength(0);
  expect(screen.getByText('有未保存修改。')).toBeInTheDocument();
});

it('点一次「继续编辑」就收起确认框，焦点回到编辑区，不发保存请求', async () => {
  const user = userEvent.setup();
  const { source, saves, onClose } = await openEditor();
  typeDraft(source, '继续写下去的草稿');
  pressEscape();
  await user.click(leaveButton('继续编辑'));
  expect(leaveDialog()).not.toBeInTheDocument();
  expect(source).toHaveFocus();
  await settle();
  expect(saves()).toHaveLength(0);
  expect(onClose).not.toHaveBeenCalled();
});

it('确认框里按 Esc 等于「继续编辑」：不关编辑器，焦点回到编辑区', async () => {
  const { source, saves, onClose } = await openEditor();
  typeDraft(source, '按了两次 Esc 的草稿');
  pressEscape();
  expect(leaveButton('继续编辑')).toHaveFocus();
  pressEscape();
  expect(leaveDialog()).not.toBeInTheDocument();
  expect(source).toHaveFocus();
  const editor = screen.getByRole('dialog', { name: '编辑与预览' });
  expect(editor).toHaveAttribute('open');
  expect(onClose).not.toHaveBeenCalled();
  await settle();
  expect(saves()).toHaveLength(0);
});

it('点一次「放弃草稿并关闭」就放弃并关闭，全程不发保存请求', async () => {
  const user = userEvent.setup();
  const { source, saves, onClose } = await openEditor();
  typeDraft(source, '要放弃的草稿');
  pressEscape();
  await user.click(leaveButton('放弃草稿并关闭'));
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.getByLabelText('内存草稿篇数')).toHaveTextContent('0');
  await settle();
  expect(saves()).toHaveLength(0);
});

it('点一次「保留内存草稿并关闭」就关闭，草稿留在内存里，不发保存请求', async () => {
  const user = userEvent.setup();
  const { source, saves, onClose } = await openEditor();
  typeDraft(source, '要留在内存里的草稿');
  pressEscape();
  await user.click(leaveButton('保留内存草稿并关闭'));
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.getByLabelText('内存草稿篇数')).toHaveTextContent('1');
  await settle();
  expect(saves()).toHaveLength(0);
});

it('点一次框里的「导出草稿与恢复信息」就导出当前草稿，框留着，不发保存请求', async () => {
  const user = userEvent.setup();
  const { source, saves, onClose } = await openEditor();
  typeDraft(source, '要导出的草稿');
  pressEscape();
  await user.click(leaveButton('导出草稿与恢复信息'));
  expect(exportDrafts).toHaveBeenCalledOnce();
  expect(vi.mocked(exportDrafts).mock.calls[0][0]).toEqual([
    expect.objectContaining({ draft: `${canonical.content}要导出的草稿` }),
  ]);
  expect(leaveDialog()).toBeInTheDocument();
  expect(onClose).not.toHaveBeenCalled();
  await settle();
  expect(saves()).toHaveLength(0);
});

it('框开着时回到正文，再点框里的说明文字或按钮：焦点都算在框内，不发保存请求', async () => {
  const user = userEvent.setup();
  const { source, saves } = await openEditor();
  typeDraft(source, '框开着时又回来写');
  pressEscape();
  const leave = screen.getByRole('alertdialog', { name: '处理未保存草稿' });
  // 回到正文用直接聚焦：真实浏览器里是点进正文，jsdom 里跑 CodeMirror 的鼠标选区没有意义。
  act(() => source.focus());
  typeDraft(source, '，再补一句');
  await user.click(within(leave).getByText(/关闭会保留本页内存草稿/));
  expect(leave).toHaveFocus();
  act(() => source.focus());
  await user.click(leaveButton('导出草稿与恢复信息'));
  expect(exportDrafts).toHaveBeenCalledOnce();
  await settle();
  expect(saves()).toHaveLength(0);
});
