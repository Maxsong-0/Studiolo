import type { EditorController, EditorSnapshot } from '../EditorController';
import { canEdit, editingExplanation } from '../canonicalModel';

interface Props {
  controller: EditorController;
  state: EditorSnapshot;
  authorized: boolean;
  dirty: boolean;
  leaving: boolean;
  onLeave: () => void;
  onContinue: () => void;
  onClose: () => void;
  onExport: () => void;
}
export function EditorNotices({
  controller,
  state,
  authorized,
  dirty,
  leaving,
  onLeave,
  onContinue,
  onClose,
  onExport,
}: Props) {
  const busy = state.phase === 'saving' || state.phase === 'verifying';
  const comparison =
    ['conflict', 'unknown', 'review', 'blocked'].includes(state.phase) &&
    state.base;
  return (
    <>
      <div className="editor-notice" aria-live="polite">
        {state.phase === 'clean' ? <p>当前没有未保存修改。</p> : null}
        {state.phase === 'dirty' ? <p>有未保存修改。</p> : null}
        {state.message ? <p>{state.message}</p> : null}
        {state.base && !canEdit(state.base) ? (
          <p>{editingExplanation(state.base)}</p>
        ) : null}
        {!authorized ? <p>当前会话未提供可信主人范围，编辑保持只读。</p> : null}
        {state.phase === 'restore' ? (
          <>
            <p>本页有尚未处理的内存草稿，请先选择恢复或放弃。</p>
            <button className="outline-button" onClick={controller.restore}>
              恢复本页草稿
            </button>
          </>
        ) : null}
        {state.phase === 'unknown' ? (
          <button
            className="outline-button"
            onClick={() => void controller.save(true)}
          >
            恢复原保存请求（同一请求标识）
          </button>
        ) : null}
        {comparison ? (
          <button
            className="outline-button"
            onClick={() => void controller.inspect()}
          >
            核对当前磁盘版本
          </button>
        ) : null}
        {state.remote && !state.pending && canEdit(state.remote) ? (
          <button
            className="outline-button"
            onClick={controller.continueFromCurrent}
          >
            以当前版本为基线继续保存草稿
          </button>
        ) : null}
        {state.base && dirty && !busy && !state.pending ? (
          <button className="text-button" data-editor-no-save onClick={onLeave}>
            放弃草稿…
          </button>
        ) : null}
      </div>
      {leaving ? (
        <section
          className="editor-leave"
          role="alertdialog"
          aria-label="处理未保存草稿"
        >
          <h3>草稿尚未处理</h3>
          <p>
            关闭会保留本页内存草稿；关闭整个页面会丢失内存内容。保存中的请求可能已经提交。
          </p>
          <button className="outline-button" onClick={onContinue}>
            继续编辑
          </button>
          <button className="outline-button" onClick={onExport}>
            导出草稿与恢复信息
          </button>
          <button className="outline-button" onClick={onClose}>
            保留内存草稿并关闭
          </button>
          {!busy && !state.pending ? (
            <button
              className="text-button"
              onClick={() => {
                controller.discard();
                onClose();
              }}
            >
              放弃草稿并关闭
            </button>
          ) : null}
        </section>
      ) : null}
    </>
  );
}
