import { useEffect, useRef } from 'react';
import { useFileOperationStore } from '../fileOperationContext';

export function FileOperationStatus({ onSettled }: { onSettled?: () => void }) {
  const { store, record } = useFileOperationStore();
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);
  if (!record) return null;
  const exportRequest = () => {
    const blob = new Blob(
      [
        JSON.stringify(
          {
            version: 1,
            request: record.request,
            restore_source: record.restoreSource,
            receipt: record.receipt ?? null,
            note: '此文件保存请求与已知回执，不能单独证明当前磁盘状态或授予写入权限。',
          },
          null,
          2,
        ),
      ],
      { type: 'application/json' },
    );
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = 'studiolo-file-operation.json';
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return (
    <section className="editor-notice" aria-label="文件操作回执">
      <p role={record.phase === 'uncertain' ? 'alert' : 'status'}>
        {record.message}
      </p>
      {record.request.kind === 'rename' ? (
        <div className="source-location">
          <p>源：{record.request.from}</p>
          <p>目标：{record.request.to}</p>
        </div>
      ) : (
        <p className="source-location">{record.request.path}</p>
      )}
      {record.phase === 'uncertain' ? (
        <p>请求仅留在本页内存。刷新或关闭页面前，请导出请求信息。</p>
      ) : null}
      <div className="history-pagination">
        {record.phase === 'uncertain' ? (
          <button
            className="outline-button"
            onClick={() => {
              const signal = lifetime.current.signal;
              void store.submit(record.request, signal).then(() => {
                if (!signal.aborted) onSettled?.();
              });
            }}
          >
            核对原请求
          </button>
        ) : null}
        <button className="outline-button" onClick={exportRequest}>
          导出请求信息
        </button>
        {['committed', 'rejected'].includes(record.phase) ? (
          <button
            className="outline-button"
            onClick={() => {
              store.dismiss();
              onSettled?.();
            }}
          >
            收起回执
          </button>
        ) : null}
      </div>
    </section>
  );
}
