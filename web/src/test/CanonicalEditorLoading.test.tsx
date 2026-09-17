import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CanonicalEditor } from '../components/CanonicalEditor';
import { DraftContext } from '../draftContext';
import { DraftStore } from '../draftStore';
import { canonical, ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';

const loading = vi.hoisted(() => {
  let releaseEditor!: () => void;
  let rejectPreview!: (error: Error) => void;
  const editor = new Promise<void>((resolve) => {
    releaseEditor = resolve;
  });
  const preview = new Promise<void>((_, reject) => {
    rejectPreview = reject;
  });
  return {
    editor,
    preview,
    releaseEditor: () => releaseEditor(),
    rejectPreview: (error: Error) => rejectPreview(error),
    editorCalls: vi.fn(),
    previewCalls: vi.fn(),
  };
});

// 推迟模块本身的载入，覆盖真正的动态 import / Suspense 边界。
vi.mock('../components/MarkdownEditor', async () => {
  loading.editorCalls();
  await loading.editor;
  return {
    MarkdownEditor: ({
      value,
      onChange,
    }: {
      value: string;
      onChange: (value: string) => void;
    }) => (
      <textarea
        aria-label="Markdown 源码"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    ),
  };
});
vi.mock('../components/MarkdownPreview', async () => {
  loading.previewCalls();
  await loading.preview;
  return { MarkdownPreview: () => null };
});

it('只读大文档不加载重库；延迟加载和预览载入失败都保留同一份最新草稿', async () => {
  let disk: typeof canonical = {
    ...canonical,
    content: '文'.repeat(90000),
    editing: { allowed: false, reason: 'too_large', max_bytes: 262144 },
  };
  const fetchMock = vi.fn(async () => jsonResponse(disk));
  vi.stubGlobal('fetch', fetchMock);
  const store = new DraftStore().forScope(ownerScope);
  const ui = () => (
    <DraftContext.Provider value={store}>
      <CanonicalEditor path={canonical.path} onClose={vi.fn()} />
    </DraftContext.Provider>
  );
  const large = render(ui());
  await screen.findByRole('region', { name: '大文档只读原文' });
  expect(loading.editorCalls).not.toHaveBeenCalled();
  expect(loading.previewCalls).not.toHaveBeenCalled();
  large.unmount();

  disk = canonical;
  store.keep({
    base: canonical,
    draft: '<script>未提交的原文</script>',
    pending: null,
  });
  render(ui());
  await waitFor(() => expect(loading.editorCalls).toHaveBeenCalledOnce());
  await waitFor(() => expect(loading.previewCalls).toHaveBeenCalledOnce());
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  fireEvent.click(await screen.findByRole('button', { name: '恢复本页草稿' }));
  const pending = screen.getByRole('region', { name: '源码编辑器加载状态' });
  expect(
    within(pending).getByText('<script>未提交的原文</script>'),
  ).toBeInTheDocument();
  expect(pending.querySelector('script')).toBeNull();
  await act(async () => loading.releaseEditor());
  const input = await screen.findByRole('textbox', { name: 'Markdown 源码' });
  expect(input).toHaveValue('<script>未提交的原文</script>');
  fireEvent.change(input, { target: { value: '加载之后继续输入' } });

  const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  try {
    await act(async () => loading.rejectPreview(new Error('虚构模块加载失败')));
    const failure = await screen.findByText(
      '编辑组件加载或渲染失败，已保留安全纯文本。',
    );
    expect(
      within(failure.parentElement!).getByText('加载之后继续输入'),
    ).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Markdown 源码' })).toBe(input);
    expect(store.get(canonical)?.draft).toBe('加载之后继续输入');
    expect(fetchMock.mock.calls).toHaveLength(2);
  } finally {
    errors.mockRestore();
  }
});
