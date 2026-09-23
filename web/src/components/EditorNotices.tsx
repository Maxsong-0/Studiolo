import type { EditorController, EditorSnapshot } from '../EditorController';
import { canEdit, editingExplanation } from '../canonicalModel';
import { Button } from '../ui';
import '../styles/editor.css';

/** 这几个阶段的附言是出了问题：保存冲突、结果不确定、被服务拒绝或核对失败。其余附言只是说明。 */
const troubled = new Set<EditorSnapshot['phase']>([
  'conflict',
  'unknown',
  'blocked',
]);

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
  const restorable = state.phase === 'restore';
  const recoverable = state.phase === 'unknown';
  const rebasable = Boolean(
    state.remote && !state.pending && canEdit(state.remote),
  );
  const discardable = Boolean(state.base && dirty && !busy && !state.pending);
  const acting =
    restorable ||
    recoverable ||
    Boolean(comparison) ||
    rebasable ||
    discardable;
  return (
    <>
      {/* 说明文字在前、动作按钮在后，顺序与改版前一致；只有出问题的附言套红色虚线框。 */}
      <div className="editor-notices" aria-live="polite">
        {state.phase === 'clean' ? <p>当前没有未保存修改。</p> : null}
        {state.phase === 'dirty' ? <p>有未保存修改。</p> : null}
        {state.message ? (
          <p
            className={
              troubled.has(state.phase) ? 'editor-notice-error' : undefined
            }
          >
            {state.message}
          </p>
        ) : null}
        {state.base && !canEdit(state.base) ? (
          <p>{editingExplanation(state.base)}</p>
        ) : null}
        {!authorized ? <p>当前会话未提供可信主人范围，编辑保持只读。</p> : null}
        {restorable ? (
          <p>本页有尚未处理的内存草稿，请先选择恢复或放弃。</p>
        ) : null}
        {acting ? (
          <div className="editor-notice-actions">
            {restorable ? (
              <Button size="sm" variant="primary" onClick={controller.restore}>
                恢复本页草稿
              </Button>
            ) : null}
            {recoverable ? (
              <Button size="sm" onClick={() => void controller.save(true)}>
                恢复原保存请求（同一请求标识）
              </Button>
            ) : null}
            {comparison ? (
              <Button size="sm" onClick={() => void controller.inspect()}>
                核对当前磁盘版本
              </Button>
            ) : null}
            {rebasable ? (
              <Button size="sm" onClick={controller.continueFromCurrent}>
                以当前版本为基线继续保存草稿
              </Button>
            ) : null}
            {discardable ? (
              <Button
                size="sm"
                variant="text"
                data-editor-no-save
                onClick={onLeave}
              >
                放弃草稿…
              </Button>
            ) : null}
          </div>
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
          <Button size="sm" onClick={onContinue}>
            继续编辑
          </Button>
          <Button size="sm" onClick={onExport}>
            导出草稿与恢复信息
          </Button>
          <Button size="sm" onClick={onClose}>
            保留内存草稿并关闭
          </Button>
          {!busy && !state.pending ? (
            <Button
              size="sm"
              variant="danger"
              onClick={() => {
                controller.discard();
                onClose();
              }}
            >
              放弃草稿并关闭
            </Button>
          ) : null}
        </section>
      ) : null}
    </>
  );
}
