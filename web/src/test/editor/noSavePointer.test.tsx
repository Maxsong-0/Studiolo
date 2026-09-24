import { act, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MarkdownEditor } from '../../components/MarkdownEditor';

// T-161 同类：Safari、macOS 上的 Firefox 与触屏点按按钮时不把焦点交给按钮，
// 编辑区失焦的 relatedTarget 为 null。按在「不保存区」里的这类点按不能触发失焦自动保存。

afterEach(() => {
  vi.useRealTimers();
});

function mount() {
  const onBlur = vi.fn();
  const view = render(
    <div>
      <MarkdownEditor
        value="草稿"
        readOnly={false}
        onChange={vi.fn()}
        canChange={() => true}
        onLimit={vi.fn()}
        onBlur={onBlur}
        onComposition={vi.fn()}
      />
      <section data-editor-no-save>
        <button type="button">放弃草稿并关闭</button>
      </section>
      <button type="button">别处</button>
    </div>,
  );
  const content = view.container.querySelector<HTMLElement>('.cm-content');
  if (!content) throw new Error('CodeMirror 没有挂载');
  const [inside, outside] = view.container.querySelectorAll('button');
  return { onBlur, content, inside, outside };
}

it('按在不保存区里、失焦没有 relatedTarget：不自动保存', () => {
  const { onBlur, content, inside } = mount();
  fireEvent.pointerDown(inside);
  fireEvent.blur(content, { relatedTarget: null });
  expect(onBlur).not.toHaveBeenCalled();
});

it('按在不保存区外、失焦没有 relatedTarget：照常自动保存', () => {
  const { onBlur, content, outside } = mount();
  fireEvent.pointerDown(outside);
  fireEvent.blur(content, { relatedTarget: null });
  expect(onBlur).toHaveBeenCalledTimes(1);
});

it('不保存区的按下只管本次点按：点完之后的失焦照常保存', () => {
  vi.useFakeTimers();
  const { onBlur, content, inside } = mount();
  fireEvent.pointerDown(inside);
  fireEvent.click(inside);
  act(() => {
    vi.runOnlyPendingTimers();
  });
  fireEvent.blur(content, { relatedTarget: null });
  expect(onBlur).toHaveBeenCalledTimes(1);
});

it('relatedTarget 给出时仍以它为准（Chrome 的点按）', () => {
  const { onBlur, content, inside, outside } = mount();
  fireEvent.pointerDown(inside);
  fireEvent.blur(content, { relatedTarget: outside });
  expect(onBlur).toHaveBeenCalledTimes(1);
  fireEvent.pointerDown(outside);
  fireEvent.blur(content, { relatedTarget: inside });
  expect(onBlur).toHaveBeenCalledTimes(1);
});
