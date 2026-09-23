import { useLayoutEffect, useRef, useState } from 'react';
import { useDraftCount } from '../draftContext';
import { exportDrafts } from '../draftExport';
import { useDialogSurface } from '../useDialogSurface';
import { Button, StatusMark, Surface } from '../ui';
import '../styles/file-operations.css';
import '../styles/editor.css';

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
      className="file-dialog draft-shelf"
      aria-labelledby="draft-title"
    >
      <header className="file-dialog__head">
        <h2 id="draft-title">本页内存草稿</h2>
        <Button size="sm" onClick={dismiss} data-dialog-dismiss>
          关闭草稿列表
        </Button>
      </header>
      <div className="file-dialog__body">
        <p>
          最多 8 篇／4
          MiB。锁定时隐藏，重新登录后可人工恢复；关闭整个页面会丢失。
        </p>
        {count ? (
          <>
            <Button size="sm" onClick={() => exportDrafts(store.list())}>
              导出全部草稿与恢复信息
            </Button>
            {/* 网格列表：一行一篇，路径、状态、打开；按钮的可读名字仍含完整路径，路径已在同一行显示。 */}
            <ul className="draft-list">
              {store.list().map((record) => (
                <li
                  key={`${record.base.vault_uuid}:${record.base.path}`}
                  className="draft-entry"
                >
                  <p className="draft-entry__path">{record.base.path}</p>
                  <p className="draft-entry__state">
                    <StatusMark state={record.pending ? 'alert' : 'pending'}>
                      {record.pending
                        ? '含结果尚未确认的保存请求'
                        : '尚未保存的草稿'}
                    </StatusMark>
                  </p>
                  <Button size="sm" onClick={() => onOpen(record.base.path)}>
                    打开草稿
                    <span className="sr-only"> {record.base.path}</span>
                  </Button>
                </li>
              ))}
            </ul>
            {confirm ? (
              <div role="alert" className="editor-leave">
                <p>
                  放弃会清空全部内存草稿，不能撤销；已发送的保存请求仍可能生效。
                </p>
                <Button size="sm" onClick={() => setConfirm(false)}>
                  保留草稿
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() => {
                    store.clear();
                    setConfirm(false);
                  }}
                >
                  确认放弃全部草稿
                </Button>
              </div>
            ) : (
              <Button size="sm" variant="text" onClick={() => setConfirm(true)}>
                放弃全部草稿…
              </Button>
            )}
          </>
        ) : (
          <p>当前没有内存草稿。</p>
        )}
      </div>
    </Surface>
  );
}
