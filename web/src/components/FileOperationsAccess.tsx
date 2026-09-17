import { lazy, Suspense, useState } from 'react';
import { useFileOperationStore } from '../fileOperationContext';
import { useFileOperations } from '../useFileOperations';
import { FileOperationStatus } from './FileOperationStatus';
const RecentlyDeleted = lazy(() =>
  import('./RecentlyDeleted').then((module) => ({
    default: module.RecentlyDeleted,
  })),
);

export function FileOperationsAccess({ onChanged }: { onChanged: () => void }) {
  const capability = useFileOperations();
  const { record } = useFileOperationStore();
  const [open, setOpen] = useState(false);
  if (capability.phase === 'disabled' && !record) return null;
  return (
    <>
      <button
        className="outline-button"
        disabled={capability.phase === 'loading'}
        onClick={() => {
          setOpen(true);
          if (capability.phase === 'error') capability.retry();
        }}
      >
        {record && ['sending', 'uncertain'].includes(record.phase)
          ? '文件操作待核对'
          : '最近删除'}
      </button>
      {open && capability.vault ? (
        <Suspense fallback={<p role="status">正在打开最近删除…</p>}>
          <RecentlyDeleted
            key={capability.vault}
            vault={capability.vault}
            onClose={() => setOpen(false)}
            onChanged={onChanged}
          />
        </Suspense>
      ) : null}
      {open && !capability.vault ? (
        <section className="file-operation-fallback" aria-label="文件操作状态">
          <p role="status">
            {capability.phase === 'disabled'
              ? '文件操作目前已关闭。'
              : '正在核对文件操作是否可用。'}
          </p>
          {capability.phase === 'error' ? (
            <button className="outline-button" onClick={capability.retry}>
              重新核对可用性
            </button>
          ) : null}
          <FileOperationStatus onSettled={onChanged} />
          <button className="outline-button" onClick={() => setOpen(false)}>
            收起
          </button>
        </section>
      ) : null}
    </>
  );
}
