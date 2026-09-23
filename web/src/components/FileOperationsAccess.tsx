import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { useFileOperationStore } from '../fileOperationContext';
import { useFileOperations } from '../useFileOperations';
import { FileOperationStatus } from './FileOperationStatus';
import { Button, Surface } from '../ui';
import '../styles/file-operations.css';
const RecentlyDeleted = lazy(() =>
  import('./RecentlyDeleted').then((module) => ({
    default: module.RecentlyDeleted,
  })),
);

/** 交给 ⌘K 动作面板的入口：等同点一次顶栏「最近删除」，以及当前能不能删文件。 */
export interface FileOperationsEntry {
  open: () => void;
  /** 文件操作可用（没被关闭）：能删才在面板里列「删除文件」预览。 */
  deletable: boolean;
}

export function FileOperationsAccess({
  onChanged,
  onEntryChange,
}: {
  onChanged: () => void;
  /** 顶栏入口出现、消失或可用性变化时回报（不显示时报 null），⌘K 据此只收录真实可用的动作。 */
  onEntryChange?: (entry: FileOperationsEntry | null) => void;
}) {
  const capability = useFileOperations();
  const { record } = useFileOperationStore();
  const [open, setOpen] = useState(false);
  const hidden = capability.phase === 'disabled' && !record;
  const deletable = capability.phase !== 'disabled';
  function show() {
    setOpen(true);
    if (capability.phase === 'error') capability.retry();
  }
  // 交出去的入口只包一层：执行时总用最新一轮的 show（可用性与重试函数每轮都会换）。
  const latestShow = useRef(show);
  useLayoutEffect(() => {
    latestShow.current = show;
  });
  useEffect(() => {
    if (!onEntryChange) return;
    onEntryChange(
      hidden ? null : { open: () => latestShow.current(), deletable },
    );
    return () => onEntryChange(null);
  }, [hidden, deletable, onEntryChange]);
  if (hidden) return null;
  return (
    <>
      <button
        className="outline-button"
        disabled={capability.phase === 'loading'}
        onClick={show}
      >
        {record && ['sending', 'uncertain'].includes(record.phase)
          ? '文件操作待核对'
          : '最近删除'}
      </button>
      {open && capability.vault ? (
        <Suspense
          fallback={
            <p role="status" className="file-operation-loading">
              正在打开最近删除…
            </p>
          }
        >
          <RecentlyDeleted
            key={capability.vault}
            vault={capability.vault}
            onClose={() => setOpen(false)}
            onChanged={onChanged}
          />
        </Suspense>
      ) : null}
      {open && !capability.vault ? (
        // 顶栏下方的小浮层：Surface 的 2px 墨框与硬阴影，出现时 120ms 透明度＋4px 位移。
        <Surface
          as="section"
          className="file-operation-fallback"
          aria-label="文件操作状态"
        >
          <p role="status">
            {capability.phase === 'disabled'
              ? '文件操作目前已关闭。'
              : '正在核对文件操作是否可用。'}
          </p>
          {capability.phase === 'error' ? (
            <Button size="sm" onClick={capability.retry}>
              重新核对可用性
            </Button>
          ) : null}
          <FileOperationStatus onSettled={onChanged} />
          <Button size="sm" onClick={() => setOpen(false)}>
            收起
          </Button>
        </Surface>
      ) : null}
    </>
  );
}
