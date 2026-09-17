import {
  lazy,
  useDeferredValue,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { EditorController } from '../EditorController';
import { byteLength, canEdit, EDIT_LIMIT } from '../canonicalModel';
import { useDraftStore } from '../draftContext';
import { exportDrafts } from '../draftExport';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { EditorPane } from './EditorPane';
import { EditorComparison } from './EditorComparison';
import { EditorNotices } from './EditorNotices';
import { PageHistory } from './PageHistory';
import type { RenameNavigation } from '../renameNavigation';
import { RenameFile } from './RenameFile';
import { DeleteFile } from './DeleteFile';
import { useFileOperationStore } from '../fileOperationContext';

// 重型输入与解析器只在打开可预览的原文后载入，状态控制器始终只有一份。
const MarkdownEditor = lazy(() =>
  import('./MarkdownEditor').then((module) => ({
    default: module.MarkdownEditor,
  })),
);
const MarkdownPreview = lazy(() =>
  import('./MarkdownPreview').then((module) => ({
    default: module.MarkdownPreview,
  })),
);

interface Props {
  path: string;
  onClose: () => void;
  expectedVault?: string;
  onRenamed?: RenameNavigation;
}
export function CanonicalEditor(props: Props) {
  const store = useDraftStore();
  return (
    <CanonicalEditorContent
      key={JSON.stringify([store.ownerScope, props.path, props.expectedVault])}
      {...props}
    />
  );
}
function CanonicalEditorContent({
  path,
  onClose,
  expectedVault,
  onRenamed,
}: Props) {
  const store = useDraftStore();
  const { store: fileOperations } = useFileOperationStore();
  const [controller] = useState(
    () =>
      new EditorController(
        path,
        store,
        () => {
          return !fileOperations.hasPending();
        },
        expectedVault,
      ),
  );
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const previewContent = useDeferredValue(state.draft);
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss: dismissEditor, captureInteraction } = useDialogSurface(
    ref,
    onClose,
  );
  const [leaving, setLeaving] = useState(false);
  const [preview, setPreview] = useState(true);
  const [historyOpen, setHistoryOpen] = useState(false);
  const historyToggle = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    void controller.load();
    return () => controller.stop();
  }, [controller]);
  const dirty = state.base && Boolean(store.get(state.base));
  const busy = state.phase === 'saving' || state.phase === 'verifying';
  const locked =
    !store.ownerScope ||
    !state.base ||
    !canEdit(state.base) ||
    state.scopeChanged ||
    state.pending?.kind === 'restore' ||
    state.phase === 'restore';
  const fileLocked =
    state.phase === 'file-operation' || fileOperations.hasPending();
  const large = byteLength(state.draft) > EDIT_LIMIT;
  const comparison =
    ['conflict', 'unknown', 'review', 'blocked'].includes(state.phase) &&
    state.base;
  const close = () => {
    if (dirty || busy) setLeaving(true);
    else dismissEditor();
  };
  const download = () => {
    const record = state.base && store.get(state.base);
    if (record) exportDrafts([record]);
  };
  return (
    <dialog
      ref={ref}
      className="document-dialog canonical-editor"
      aria-labelledby="editor-title"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <header className="document-header">
        <div>
          <h2 id="editor-title">编辑与预览</h2>
          <p className="source-location">{path}</p>
        </div>
        <button
          className="icon-button"
          onClick={close}
          aria-label="关闭编辑器"
          data-dialog-dismiss
          data-editor-no-save
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="editor-controls">
        <button
          className="outline-button"
          aria-pressed={preview}
          onClick={() => setPreview((value) => !value)}
        >
          {preview ? '隐藏预览' : '显示预览'}
        </button>
        <button
          className="outline-button"
          disabled={state.phase !== 'dirty' || locked || fileLocked}
          onClick={() => void controller.save()}
        >
          立即保存
        </button>
        <button className="outline-button" disabled={!dirty} onClick={download}>
          导出草稿与恢复信息
        </button>
        <button
          ref={historyToggle}
          className="outline-button"
          data-editor-no-save
          disabled={!state.base || state.scopeChanged}
          aria-expanded={historyOpen && !state.scopeChanged}
          onClick={() => setHistoryOpen((open) => !open)}
        >
          {historyOpen && !state.scopeChanged ? '隐藏页面历史' : '页面历史'}
        </button>
        <span>默认软换行 · 停止输入 1.5 秒或失焦后自动保存</span>
      </div>
      <EditorNotices
        controller={controller}
        state={state}
        authorized={Boolean(store.ownerScope)}
        dirty={Boolean(dirty)}
        leaving={leaving}
        onLeave={() => setLeaving(true)}
        onClose={dismissEditor}
        onExport={download}
        onContinue={() => {
          setLeaving(false);
          ref.current?.querySelector<HTMLElement>('.cm-content')?.focus();
        }}
      />
      <DeleteFile controller={controller} state={state} />
      <RenameFile
        controller={controller}
        state={state}
        onRenamed={
          onRenamed
            ? (request, receipt, signal, isCurrent) => {
                const interactionCurrent = captureInteraction();
                return onRenamed(
                  request,
                  receipt,
                  signal,
                  () => interactionCurrent() && isCurrent(),
                );
              }
            : undefined
        }
      />
      {historyOpen && state.base && !state.scopeChanged ? (
        <PageHistory
          key={JSON.stringify([
            store.ownerScope,
            state.base.vault_uuid,
            state.base.path,
            state.base.revision,
          ])}
          controller={controller}
          state={state}
          onRestoreStart={() => historyToggle.current?.focus()}
          onClose={() => {
            setHistoryOpen(false);
            historyToggle.current?.focus();
          }}
        />
      ) : null}
      {state.base ? (
        <div className="editor-scroll">
          {large ? (
            <section className="editor-notice" aria-label="大文档只读原文">
              <pre>{state.draft}</pre>
            </section>
          ) : (
            <div
              className={`editor-panes${preview ? '' : ' editor-source-only'}`}
            >
              <section aria-label="源码编辑区域">
                <EditorPane content={state.draft} label="源码编辑器">
                  <MarkdownEditor
                    value={state.draft}
                    readOnly={locked || fileLocked}
                    onChange={controller.change}
                    canChange={controller.canChange}
                    onLimit={controller.limit}
                    onBlur={controller.blur}
                    onComposition={controller.composition}
                  />
                </EditorPane>
              </section>
              {preview ? (
                <EditorPane content={state.draft} label="Markdown 预览">
                  <MarkdownPreview
                    content={previewContent}
                    updating={previewContent !== state.draft}
                  />
                </EditorPane>
              ) : null}
            </div>
          )}
          {comparison ? <EditorComparison state={state} /> : null}
        </div>
      ) : (
        <div className="editor-notice">
          <p>
            {state.phase === 'loading'
              ? '正在读取权威文档…'
              : '未取得权威文档，不能启用编辑。'}
          </p>
          {state.phase !== 'loading' ? (
            <button
              className="outline-button"
              onClick={() => void controller.load()}
            >
              重新读取权威文档
            </button>
          ) : null}
        </div>
      )}
    </dialog>
  );
}
