import { vi } from 'vitest';
import { CanonicalEditor } from '../components/CanonicalEditor';
import { DraftContext } from '../draftContext';
import type { DraftStore } from '../draftStore';
import { FileOperationContext } from '../fileOperationContext';
import type { FileOperationStore } from '../fileOperationStore';
import { useWorkbench } from '../useWorkbench';
import { useEditorNavigation } from '../useEditorNavigation';
import { canonical } from './canonicalFixtures';

// 只替换重型输入视图；新旧 Controller、能力发现、Store、刷新及导航均使用实际实现。
vi.mock('../components/MarkdownEditor', () => ({
  MarkdownEditor: ({
    value,
    readOnly,
    onChange,
  }: {
    value: string;
    readOnly: boolean;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown 源码"
      value={value}
      readOnly={readOnly}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));
function Surface() {
  const { refreshLatest } = useWorkbench();
  const editor = useEditorNavigation(refreshLatest);
  return (
    <>
      <button onClick={() => editor.open(canonical.path)}>打开源</button>
      <output aria-label="已打开路径">
        {editor.current?.path ?? 'closed'}
      </output>
      {editor.current ? (
        <CanonicalEditor
          key={editor.current.id}
          path={editor.current.path}
          expectedVault={editor.current.vault}
          onClose={() => editor.open(null)}
          onRenamed={editor.onRenamed}
        />
      ) : null}
    </>
  );
}
export function CanonicalRenameHarness({
  drafts,
  files,
}: {
  drafts: DraftStore;
  files: FileOperationStore;
}) {
  return (
    <DraftContext.Provider value={drafts}>
      <FileOperationContext.Provider value={files}>
        <Surface />
      </FileOperationContext.Provider>
    </DraftContext.Provider>
  );
}
