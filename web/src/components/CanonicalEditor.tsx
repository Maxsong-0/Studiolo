import {
  lazy,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { EditorController } from '../EditorController';
import type { EditorSnapshot } from '../EditorController';
import { byteLength, canEdit, EDIT_LIMIT } from '../canonicalModel';
import type { CanonicalDocument } from '../canonicalModel';
import { freshnessReason } from '../freshness';
import { useDraftStore } from '../draftContext';
import { exportDrafts } from '../draftExport';
import { useDialogSurface } from '../useDialogSurface';
import {
  Button,
  Cell,
  EmptyState,
  GridFrame,
  StatusMark,
  Surface,
  type StatusState,
} from '../ui';
import { Icon } from './Icon';
import { EditorPane } from './EditorPane';
import { EditorComparison } from './EditorComparison';
import { EditorNotices } from './EditorNotices';
import { PageHistory } from './PageHistory';
import type { RenameNavigation } from '../renameNavigation';
import { RenameFile } from './RenameFile';
import { DeleteFile } from './DeleteFile';
import { useFileOperationStore } from '../fileOperationContext';
import '../styles/editor.css';

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

interface Mark {
  state: StatusState;
  label: string;
}

/**
 * 状态行「保存」一格：把编辑状态机的阶段换成等宽短标签与方块（规范第 1 节）。
 * ■ 与基线一致、□ 待定、黑 ■ 在跑、红 ■ 出问题或等你选择、灰虚线 只读；文字始终写出来。
 */
function saveMark(phase: EditorSnapshot['phase'], readOnly: boolean): Mark {
  switch (phase) {
    case 'loading':
      return { state: 'pending', label: '读取中' };
    case 'clean':
      return readOnly
        ? { state: 'dormant', label: '只读' }
        : { state: 'ok', label: '无改动' };
    case 'dirty':
      return { state: 'pending', label: '未保存' };
    case 'saving':
      return { state: 'active', label: '保存中' };
    case 'verifying':
      return { state: 'active', label: '核对磁盘' };
    case 'conflict':
      return { state: 'alert', label: '版本冲突' };
    case 'unknown':
      return { state: 'alert', label: '结果未知' };
    case 'blocked':
      return { state: 'alert', label: '已阻断' };
    case 'restore':
      return { state: 'alert', label: '草稿待处理' };
    case 'review':
      return { state: 'alert', label: '待比较' };
    case 'file-operation':
      return { state: 'pending', label: '已暂停' };
  }
}

/**
 * 状态行「新鲜度」一格：沿用新鲜度提示的原话；陈旧的原因进 title。
 * 没有证据与陈旧一样按出问题标红：未知不等于正常，编辑也因此保持只读（与状态胶囊同一口径）。
 */
function freshnessMark(
  document: CanonicalDocument,
): Mark & { reason?: string } {
  const { freshness } = document;
  if (!freshness)
    return {
      state: 'alert',
      label: '新鲜度未知',
      reason: '当前服务未提供 canonical 新鲜度证据。',
    };
  if (freshness.stale)
    return {
      state: 'alert',
      label: '快照已陈旧',
      reason: freshnessReason(freshness),
    };
  return { state: 'ok', label: '已追平已知提交' };
}

/** 读数格：拿不到时显示「—」，原因写进 title 并给读屏一份，绝不补一个值。 */
function Reading({
  label,
  value,
  reason = '',
  title,
}: {
  label: string;
  value: ReactNode | null;
  /** value 为 null 时必须给：为什么拿不到。 */
  reason?: string;
  /** 值完整的说法（如版本全长），显示被截短时给。 */
  title?: string;
}) {
  const missing = value === null;
  return (
    <div>
      <dt>{label}</dt>
      <dd title={missing ? reason : title}>
        {missing ? (
          <>
            —<span className="sr-only">{reason}</span>
          </>
        ) : (
          value
        )}
      </dd>
    </div>
  );
}

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
  // 离开确认框开着时，人正在决定这份草稿的去留：停顿计时不再发起自动保存，
  // 免得保存抢在「放弃」「保留并关闭」之前提交，或在点击途中改动版面。「继续编辑」后恢复。
  useEffect(() => {
    controller.holdAutosave(leaving);
  }, [controller, leaving]);
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
  const continueEditing = () => {
    setLeaving(false);
    ref.current?.querySelector<HTMLElement>('.cm-content')?.focus();
  };
  // 关闭请求（Esc、返回手势、requestClose()）：确认框开着就等于「继续编辑」，否则同关闭按钮。
  const requestClose = () => {
    if (leaving) continueEditing();
    else close();
  };
  const download = () => {
    const record = state.base && store.get(state.base);
    if (record) exportDrafts([record]);
  };
  // 其余关闭请求走 dialog 的 cancel：拦下浏览器的直接关闭，改走同一个 requestClose。
  // 不可取消的 cancel 是浏览器坚持要关（拦下一次之后没有新的用户激活，如返回手势连按），拦不住；
  // 这里不动，由 useDialogSurface 跟着 close 收起：本页草稿留在内存里，与「保留内存草稿并关闭」相同，绝不丢。
  // 原生监听只挂一次，调用时总用最新一轮的 requestClose。
  const latestRequestClose = useRef(requestClose);
  useLayoutEffect(() => {
    latestRequestClose.current = requestClose;
  });
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    function cancel(event: Event) {
      event.preventDefault();
      if (event.cancelable) latestRequestClose.current();
    }
    element.addEventListener('cancel', cancel);
    return () => element.removeEventListener('cancel', cancel);
  }, []);
  const save = saveMark(state.phase, Boolean(locked || fileLocked));
  const freshness = state.base ? freshnessMark(state.base) : null;
  const watermark = state.base?.freshness?.committed_through ?? null;
  return (
    <Surface
      as="dialog"
      ref={ref}
      appear={false}
      className="editor-overlay"
      aria-labelledby="editor-title"
      onKeyDown={(event) => {
        // Esc 在 keydown 阶段拦下、走 requestClose：不再变成浏览器的关闭请求，
        // 连按也就不会被判为不可取消而强行关掉编辑器。编辑器自己用掉的（收起补全、历史确认框等）、
        // 输入法组字中的、落在别的对话框里的 Esc 不管；焦点在离开确认框里时由框自己拦下，到不了这里。
        if (
          event.key !== 'Escape' ||
          event.defaultPrevented ||
          event.nativeEvent.isComposing ||
          !(event.target instanceof Element) ||
          event.target.closest('dialog') !== event.currentTarget
        )
          return;
        event.preventDefault();
        event.stopPropagation();
        requestClose();
      }}
    >
      <header className="editor-titlebar">
        <div className="editor-titlebar__row">
          <h2 id="editor-title">编辑与预览</h2>
          <button
            className="icon-button"
            onClick={close}
            aria-label="关闭编辑器"
            data-dialog-dismiss
            data-editor-no-save
          >
            <Icon name="close" />
          </button>
        </div>
        <dl className="editor-status">
          <Reading label="路径" value={path} />
          <Reading
            label="版本"
            value={state.base ? `${state.base.revision.slice(0, 12)}…` : null}
            title={state.base?.revision}
            reason="尚未取得权威文档，没有编辑基线版本。"
          />
          <Reading
            label="提交水位"
            value={watermark}
            reason={
              !state.base
                ? '尚未取得权威文档。'
                : !state.base.freshness
                  ? '当前服务未提供 canonical 新鲜度证据。'
                  : '服务未报告提交水位。'
            }
          />
          <Reading
            label="新鲜度"
            value={
              freshness ? (
                <StatusMark state={freshness.state}>
                  {freshness.label}
                </StatusMark>
              ) : null
            }
            title={freshness?.reason}
            reason="尚未取得权威文档。"
          />
          <Reading
            label="保存"
            value={<StatusMark state={save.state}>{save.label}</StatusMark>}
          />
        </dl>
      </header>
      <div className="editor-controls">
        <Button
          size="sm"
          aria-pressed={preview}
          onClick={() => setPreview((value) => !value)}
        >
          {preview ? '隐藏预览' : '显示预览'}
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={state.phase !== 'dirty' || locked || fileLocked}
          onClick={() => void controller.save()}
        >
          立即保存
        </Button>
        <Button size="sm" disabled={!dirty} onClick={download}>
          导出草稿与恢复信息
        </Button>
        <Button
          ref={historyToggle}
          size="sm"
          data-editor-no-save
          disabled={!state.base || state.scopeChanged}
          aria-expanded={historyOpen && !state.scopeChanged}
          onClick={() => setHistoryOpen((open) => !open)}
        >
          {historyOpen && !state.scopeChanged ? '隐藏页面历史' : '页面历史'}
        </Button>
        <span className="editor-controls__hint">
          默认软换行 · 停止输入 1.5 秒或失焦后自动保存
        </span>
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
        onContinue={continueEditing}
      />
      <div className="editor-file-ops">
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
      </div>
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
            <section className="editor-large" aria-label="大文档只读原文">
              <pre>{state.draft}</pre>
            </section>
          ) : (
            <GridFrame columns={preview ? 2 : 1} className="editor-panes">
              <Cell
                as="section"
                span={1}
                flush
                className="editor-pane--source"
                aria-label="源码编辑区域"
              >
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
              </Cell>
              {preview ? (
                <Cell as="div" span={1} className="editor-pane--preview">
                  <EditorPane content={state.draft} label="Markdown 预览">
                    <MarkdownPreview
                      content={previewContent}
                      updating={previewContent !== state.draft}
                    />
                  </EditorPane>
                </Cell>
              ) : null}
            </GridFrame>
          )}
          {comparison ? <EditorComparison state={state} /> : null}
        </div>
      ) : (
        <EmptyState
          className="editor-placeholder"
          title={
            state.phase === 'loading'
              ? '正在读取权威文档…'
              : '未取得权威文档，不能启用编辑。'
          }
          action={
            state.phase !== 'loading' ? (
              <Button size="sm" onClick={() => void controller.load()}>
                重新读取权威文档
              </Button>
            ) : null
          }
        />
      )}
    </Surface>
  );
}
