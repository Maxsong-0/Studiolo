import { useEffect, useRef } from 'react';
import { useFileOperationStore } from '../fileOperationContext';
import { Button } from '../ui';
import { cx } from '../ui/dom';
import '../styles/file-operations.css';

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
  // 结果未知与明确未执行都算出了问题：红色虚线框＋红色方块；等回执是空心方块，已提交是实心方块。
  const trouble = record.phase === 'uncertain' || record.phase === 'rejected';
  return (
    <section
      className={cx('file-receipt', trouble && 'file-receipt--trouble')}
      aria-label="文件操作回执"
    >
      <p
        className="file-receipt__message"
        data-phase={record.phase}
        role={record.phase === 'uncertain' ? 'alert' : 'status'}
      >
        {record.message}
      </p>
      {record.request.kind === 'rename' ? (
        <div className="file-receipt__paths">
          <p>源：{record.request.from}</p>
          <p>目标：{record.request.to}</p>
        </div>
      ) : (
        <p className="file-receipt__paths">{record.request.path}</p>
      )}
      {record.phase === 'uncertain' ? (
        <p>请求仅留在本页内存。刷新或关闭页面前，请导出请求信息。</p>
      ) : null}
      <div className="file-receipt__actions">
        {record.phase === 'uncertain' ? (
          <Button
            size="sm"
            onClick={() => {
              const signal = lifetime.current.signal;
              void store.submit(record.request, signal).then(() => {
                if (!signal.aborted) onSettled?.();
              });
            }}
          >
            核对原请求
          </Button>
        ) : null}
        <Button size="sm" onClick={exportRequest}>
          导出请求信息
        </Button>
        {['committed', 'rejected'].includes(record.phase) ? (
          <Button
            size="sm"
            onClick={() => {
              store.dismiss();
              onSettled?.();
            }}
          >
            收起回执
          </Button>
        ) : null}
      </div>
    </section>
  );
}
