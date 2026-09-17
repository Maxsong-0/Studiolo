import { useEffect, useRef, useState } from 'react';
import type { EditorController, EditorSnapshot } from '../EditorController';
import { useFileOperationStore } from '../fileOperationContext';
import { useFileOperations } from '../useFileOperations';
import { FileOperationStatus } from './FileOperationStatus';

export function DeleteFile({
  controller,
  state,
}: {
  controller: EditorController;
  state: EditorSnapshot;
}) {
  const { store, record } = useFileOperationStore();
  const capability = useFileOperations();
  const [reviewing, setReviewing] = useState(false);
  const [confirmedState, setConfirmedState] = useState<EditorSnapshot | null>(
    null,
  );
  const [reviewedRevision, setReviewedRevision] = useState<string | null>(null);
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);
  const base = state.base;
  const allowed = Boolean(
    base &&
    capability.vault === base.vault_uuid &&
    controller.canRestoreHistory() &&
    store.canStart(),
  );
  if (!base || capability.phase === 'disabled') return null;
  const matchesReview = reviewedRevision === base.revision;
  // 任一次编辑状态变化都会使已勾选的审阅失效，即使之后又回到相同版本。
  const confirmed = confirmedState === state;
  return (
    <section className="file-delete" data-editor-no-save aria-label="删除文件">
      {!record && !store.canStart() ? (
        <p role="status">本页文件操作记录已满，请先处理已有记录。</p>
      ) : null}
      <button
        className="outline-button"
        disabled={!allowed}
        aria-expanded={reviewing}
        onClick={() => {
          setReviewing((value) => !value);
          setConfirmedState(null);
          setReviewedRevision(base.revision);
        }}
      >
        删除文件
      </button>
      {reviewing ? (
        <div className="editor-notice">
          <p>
            将删除当前已保存版本：
            <strong className="source-location">{base.path}</strong>
          </p>
          <p>
            需要先保存或处理草稿。删除后可在「最近删除」恢复原路径；若原路径已被占用，恢复会停止。
          </p>
          {!matchesReview ? (
            <p>已保存版本发生变化，请重新展开删除确认。</p>
          ) : null}
          <label>
            <input
              type="checkbox"
              checked={confirmed && matchesReview && allowed}
              disabled={!allowed || !matchesReview}
              onChange={(event) =>
                setConfirmedState(event.target.checked ? state : null)
              }
            />
            我确认删除这个已保存版本
          </label>
          <button
            className="outline-button"
            disabled={!allowed || !confirmed || !matchesReview}
            onClick={() => {
              if (!reviewedRevision) return;
              const current = controller.beginFileOperation(reviewedRevision);
              if (!current) return;
              setConfirmedState(null);
              void store.submit(
                {
                  kind: 'delete',
                  vault_uuid: current.vault_uuid,
                  path: current.path,
                  expected_revision: current.revision,
                  idempotency_key: crypto.randomUUID(),
                },
                lifetime.current.signal,
              );
            }}
          >
            确认删除
          </button>
        </div>
      ) : null}
      {record &&
      record.request.kind !== 'rename' &&
      record.request.path === base.path &&
      record.request.vault_uuid === base.vault_uuid ? (
        <FileOperationStatus />
      ) : null}
    </section>
  );
}
