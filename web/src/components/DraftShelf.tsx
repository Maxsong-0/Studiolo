import { useRef, useState } from 'react';
import { useDraftCount } from '../draftContext';
import { exportDrafts } from '../draftExport';
import { useDialogSurface } from '../useDialogSurface';

export function DraftShelf({
  onOpen,
  onClose,
}: {
  onOpen: (path: string) => void;
  onClose: () => void;
}) {
  const { store, count } = useDraftCount();
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, onClose);
  const [confirm, setConfirm] = useState(false);
  return (
    <dialog
      ref={ref}
      className="session-dialog draft-shelf"
      aria-labelledby="draft-title"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <header className="session-dialog-header">
        <h2 id="draft-title">本页内存草稿</h2>
        <button
          className="outline-button"
          onClick={dismiss}
          data-dialog-dismiss
        >
          关闭草稿列表
        </button>
      </header>
      <div className="session-dialog-body">
        <p>
          最多 8 篇／4
          MiB。锁定时隐藏，重新登录后可人工恢复；关闭整个页面会丢失。
        </p>
        {count ? (
          <>
            <button
              className="outline-button"
              onClick={() => exportDrafts(store.list())}
            >
              导出全部草稿与恢复信息
            </button>
            <ul>
              {store.list().map((record) => (
                <li key={`${record.base.vault_uuid}:${record.base.path}`}>
                  <p>{record.base.path}</p>
                  <p>
                    {record.pending
                      ? '含结果尚未确认的保存请求'
                      : '尚未保存的草稿'}
                  </p>
                  <button
                    className="outline-button"
                    onClick={() => onOpen(record.base.path)}
                  >
                    打开草稿 {record.base.path}
                  </button>
                </li>
              ))}
            </ul>
            {confirm ? (
              <div role="alert">
                <p>
                  放弃会清空全部内存草稿，不能撤销；已发送的保存请求仍可能生效。
                </p>
                <button
                  className="outline-button"
                  onClick={() => setConfirm(false)}
                >
                  保留草稿
                </button>
                <button
                  className="text-button"
                  onClick={() => {
                    store.clear();
                    setConfirm(false);
                  }}
                >
                  确认放弃全部草稿
                </button>
              </div>
            ) : (
              <button className="text-button" onClick={() => setConfirm(true)}>
                放弃全部草稿…
              </button>
            )}
          </>
        ) : (
          <p>当前没有内存草稿。</p>
        )}
      </div>
    </dialog>
  );
}
