import { act, fireEvent, render, screen } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import { undo } from '@codemirror/commands';
import { afterEach, expect, it, vi } from 'vitest';
import { MarkdownEditor } from '../components/MarkdownEditor';

afterEach(() => {
  document
    .querySelectorAll('meta[name="studiolo-style-nonce"]')
    .forEach((meta) => meta.remove());
});

it.each([
  ['a'.repeat(64), 'a'.repeat(64)],
  ['__STUDIOLO_STYLE_NONCE__', ''],
  ['A'.repeat(64), ''],
  ['a'.repeat(63), ''],
  ['a'.repeat(64) + '\n', ''],
  [null, ''],
])('样式 nonce %s 只接受可信 HTML 头中的完整小写摘要', (value, expected) => {
  if (value !== null) {
    const meta = document.createElement('meta');
    meta.name = 'studiolo-style-nonce';
    meta.content = value;
    document.head.append(meta);
  }
  const bodyMeta = document.createElement('meta');
  bodyMeta.name = 'studiolo-style-nonce';
  bodyMeta.content = 'b'.repeat(64);
  document.body.append(bodyMeta);
  render(
    <MarkdownEditor
      value="只读文本"
      readOnly
      onChange={vi.fn()}
      canChange={() => true}
      onLimit={vi.fn()}
      onBlur={vi.fn()}
      onComposition={vi.fn()}
    />,
  );
  const view = EditorView.findFromDOM(
    screen.getByRole('textbox', { name: 'Markdown 源码' }),
  )!;
  expect(view.state.facet(EditorView.cspNonce)).toBe(expected);
  expect(view.state.sliceDoc()).toBe('只读文本');
});

it('实际 CodeMirror 保留原始换行/未知frontmatter，单内核支持撤销', () => {
  const value = '---\r\n# 注释\r\n未知: 保留\r\n---\r\n正文';
  const onChange = vi.fn();
  const callbacks = {
    onChange,
    canChange: () => true,
    onLimit: vi.fn(),
    onBlur: vi.fn(),
    onComposition: vi.fn(),
  };
  const { container, rerender, unmount } = render(
    <MarkdownEditor value={value} readOnly={false} {...callbacks} />,
  );
  const content = screen.getByRole('textbox', { name: 'Markdown 源码' });
  const view = EditorView.findFromDOM(content)!;
  expect(view.state.sliceDoc()).toBe(value);
  expect(view.lineWrapping).toBe(true);
  act(() =>
    view.dispatch({ changes: { from: view.state.doc.length, insert: '继续' } }),
  );
  expect(onChange).toHaveBeenLastCalledWith(`${value}继续`);
  rerender(
    <MarkdownEditor value={`${value}继续`} readOnly={false} {...callbacks} />,
  );
  expect(EditorView.findFromDOM(content)).toBe(view);
  expect(container.querySelectorAll('.cm-editor')).toHaveLength(1);
  act(() => {
    undo(view);
  });
  expect(onChange).toHaveBeenLastCalledWith(value);
  fireEvent.compositionStart(content);
  expect(callbacks.onComposition).toHaveBeenCalledWith(true);
  unmount();
  expect(container.querySelector('.cm-editor')).toBeNull();
});
it('超限输入在事务前拒绝并说明原因，原有文本不受损', async () => {
  const onLimit = vi.fn();
  const onChange = vi.fn();
  render(
    <MarkdownEditor
      value="保留"
      readOnly={false}
      onChange={onChange}
      canChange={() => false}
      onLimit={onLimit}
      onBlur={vi.fn()}
      onComposition={vi.fn()}
    />,
  );
  const view = EditorView.findFromDOM(
    screen.getByRole('textbox', { name: 'Markdown 源码' }),
  )!;
  await act(async () => {
    view.dispatch({ changes: { from: 0, insert: '超限' } });
  });
  expect(view.state.sliceDoc()).toBe('保留');
  expect(onChange).not.toHaveBeenCalled();
  expect(onLimit).toHaveBeenCalledOnce();
});
