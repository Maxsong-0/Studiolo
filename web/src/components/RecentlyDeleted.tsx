import { useCallback, useEffect, useRef, useState } from 'react';
import { readTombstones } from '../fileOperationApi';
import { useFileOperationStore } from '../fileOperationContext';
import type { TombstoneEntry, TombstonePage } from '../fileOperationModel';
import { useDialogSurface } from '../useDialogSurface';
import { FileOperationStatus } from './FileOperationStatus';

export function RecentlyDeleted({
  vault,
  onClose,
  onChanged,
}: {
  vault: string;
  onClose: () => void;
  onChanged: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, onClose);
  const { store, record } = useFileOperationStore();
  const [page, setPage] = useState<TombstonePage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<TombstoneEntry | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const reader = useRef<AbortController | null>(null);
  const operation = useRef(new AbortController());
  const load = useCallback(
    async (cursor: string | null) => {
      if (operation.current.signal.aborted) return;
      reader.current?.abort();
      const controller = new AbortController();
      reader.current = controller;
      setLoading(true);
      setError('');
      setSelected(null);
      setConfirmed(false);
      try {
        const next = await readTombstones(vault, cursor, controller.signal);
        if (!controller.signal.aborted) setPage(next);
      } catch (failure) {
        if (!controller.signal.aborted) {
          setPage(null);
          setError(
            failure instanceof Error
              ? failure.message
              : '删除记录暂时无法核对。',
          );
        }
      } finally {
        if (!controller.signal.aborted) setLoading(false);
      }
    },
    [vault],
  );
  useEffect(() => {
    operation.current = new AbortController();
    void load(null);
    return () => {
      reader.current?.abort();
      operation.current.abort();
    };
  }, [load]);
  return (
    <dialog
      ref={ref}
      className="document-dialog recent-deleted"
      aria-labelledby="deleted-title"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <header className="document-header">
        <div>
          <h2 id="deleted-title">最近删除</h2>
          <p>恢复到原路径。若已有同名文件，恢复会停止。</p>
        </div>
        <button className="outline-button" onClick={dismiss}>
          关闭
        </button>
      </header>
      <div className="deleted-scroll">
        <FileOperationStatus
          onSettled={() => {
            onChanged();
            void load(null);
          }}
        />
        <div className="history-pagination">
          <button
            className="outline-button"
            disabled={loading || record?.phase === 'sending'}
            onClick={() => void load(null)}
          >
            重新读取第一页
          </button>
          <button
            className="outline-button"
            disabled={
              loading || !page?.next_cursor || record?.phase === 'sending'
            }
            onClick={() => {
              if (page?.next_cursor) void load(page.next_cursor);
            }}
          >
            下一页
          </button>
        </div>
        {loading ? <p role="status">正在核对删除记录…</p> : null}
        {error ? <p role="alert">{error} 请重新读取第一页。</p> : null}
        {!record && !store.canStart() ? (
          <p role="status">本页文件操作记录已满，请先处理已有记录。</p>
        ) : null}
        {page?.entries.length === 0 ? (
          <p>
            {page.next_cursor
              ? '这一页没有范围内的记录，可以继续下一页。'
              : '这一页没有可恢复的删除记录。'}
          </p>
        ) : null}
        <ul className="history-entries">
          {page?.entries.map((entry) => (
            <li key={entry.id}>
              <strong className="source-location">{entry.path}</strong>
              <p>
                {entry.bytes.toLocaleString()} 字节 ·{' '}
                {entry.deleted_at_ms === null
                  ? '删除时间未记录'
                  : new Date(entry.deleted_at_ms).toLocaleString()}
              </p>
              <button
                className="outline-button"
                disabled={loading || !store.canStart()}
                aria-pressed={selected?.id === entry.id}
                onClick={() => {
                  setSelected(entry);
                  setConfirmed(false);
                }}
              >
                恢复这个文件
              </button>
            </li>
          ))}
        </ul>
        {selected ? (
          <section className="editor-notice" aria-label="确认恢复删除文件">
            <p>
              恢复原路径：
              <strong className="source-location">{selected.path}</strong>
            </p>
            <label>
              <input
                type="checkbox"
                checked={confirmed}
                disabled={!store.canStart()}
                onChange={(event) => setConfirmed(event.target.checked)}
              />
              我确认恢复所选删除版本
            </label>
            <button
              className="outline-button"
              disabled={!confirmed || !store.canStart()}
              onClick={() => {
                setConfirmed(false);
                const { id, path, revision, sha256, bytes } = selected;
                const signal = operation.current.signal;
                void store
                  .submit(
                    {
                      kind: 'restore_tombstone',
                      vault_uuid: vault,
                      path,
                      tombstone_id: id,
                      idempotency_key: crypto.randomUUID(),
                    },
                    signal,
                    { id, path, revision, sha256, bytes },
                  )
                  .then(() => {
                    if (signal.aborted) return;
                    if (store.getSnapshot()?.phase === 'committed') {
                      onChanged();
                      void load(null);
                    }
                  });
              }}
            >
              确认恢复
            </button>
          </section>
        ) : null}
      </div>
    </dialog>
  );
}
