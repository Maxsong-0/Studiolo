import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { readTombstones } from '../fileOperationApi';
import { useFileOperationStore } from '../fileOperationContext';
import type { TombstoneEntry, TombstonePage } from '../fileOperationModel';
import { useDialogSurface } from '../useDialogSurface';
import { Button, Surface } from '../ui';
import { FileOperationStatus } from './FileOperationStatus';
import '../styles/file-operations.css';

/** 删除时间：本地时间原样（toLocaleString），机器可读的 ISO 放进 dateTime；超出 Date 范围时不造一个。 */
function DeletedAt({ ms }: { ms: number }) {
  const date = new Date(ms);
  const valid = Number.isFinite(date.getTime());
  return (
    <time dateTime={valid ? date.toISOString() : undefined}>
      {date.toLocaleString()}
    </time>
  );
}

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
  // Esc 走 dialog 的 cancel：拦下浏览器的直接关闭，改走可中断的收起并归还焦点。
  // Surface 的属性类型只收通用 HTML 属性，所以用原生监听，紧跟 showModal 挂上。
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    function cancel(event: Event) {
      event.preventDefault();
      dismiss();
    }
    element.addEventListener('cancel', cancel);
    return () => element.removeEventListener('cancel', cancel);
  }, [dismiss]);
  return (
    <Surface
      as="dialog"
      ref={ref}
      appear={false}
      className="file-dialog recent-deleted"
      aria-labelledby="deleted-title"
    >
      <header className="file-dialog__head">
        <div>
          <h2 id="deleted-title">最近删除</h2>
          <p>恢复到原路径。若已有同名文件，恢复会停止。</p>
        </div>
        <Button size="sm" onClick={dismiss}>
          关闭
        </Button>
      </header>
      <div className="file-dialog__body">
        <FileOperationStatus
          onSettled={() => {
            onChanged();
            void load(null);
          }}
        />
        <div className="deleted-pagination">
          <Button
            size="sm"
            disabled={loading || record?.phase === 'sending'}
            onClick={() => void load(null)}
          >
            重新读取第一页
          </Button>
          <Button
            size="sm"
            disabled={
              loading || !page?.next_cursor || record?.phase === 'sending'
            }
            onClick={() => {
              if (page?.next_cursor) void load(page.next_cursor);
            }}
          >
            下一页
          </Button>
        </div>
        {loading ? (
          <p role="status" className="file-dialog__note">
            正在核对删除记录…
          </p>
        ) : null}
        {error ? (
          <p role="alert" className="file-dialog__error">
            {error} 请重新读取第一页。
          </p>
        ) : null}
        {!record && !store.canStart() ? (
          <p role="status" className="file-dialog__note">
            本页文件操作记录已满，请先处理已有记录。
          </p>
        ) : null}
        {page?.entries.length === 0 ? (
          <p>
            {page.next_cursor
              ? '这一页没有范围内的记录，可以继续下一页。'
              : '这一页没有可恢复的删除记录。'}
          </p>
        ) : null}
        <ul className="deleted-list">
          {page?.entries.map((entry) => (
            <li key={entry.id} className="deleted-entry">
              <strong className="deleted-entry__path">{entry.path}</strong>
              <p className="deleted-entry__meta">
                {entry.bytes.toLocaleString()} 字节 ·{' '}
                {entry.deleted_at_ms === null ? (
                  '删除时间未记录'
                ) : (
                  <DeletedAt ms={entry.deleted_at_ms} />
                )}
              </p>
              <Button
                size="sm"
                disabled={loading || !store.canStart()}
                aria-pressed={selected?.id === entry.id}
                onClick={() => {
                  setSelected(entry);
                  setConfirmed(false);
                }}
              >
                恢复这个文件
              </Button>
            </li>
          ))}
        </ul>
        {selected ? (
          <Surface
            as="section"
            className="file-op__review"
            aria-label="确认恢复删除文件"
          >
            <p>
              恢复原路径：
              <strong className="file-op__path">{selected.path}</strong>
            </p>
            <label className="file-op__confirm">
              <input
                type="checkbox"
                checked={confirmed}
                disabled={!store.canStart()}
                onChange={(event) => setConfirmed(event.target.checked)}
              />
              我确认恢复所选删除版本
            </label>
            <div className="file-op__actions">
              <Button
                size="sm"
                variant="primary"
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
              </Button>
            </div>
          </Surface>
        ) : null}
      </div>
    </Surface>
  );
}
