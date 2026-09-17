import { useCallback, useEffect, useRef, useState } from 'react';
import type { EditorController, EditorSnapshot } from './EditorController';
import type {
  FileOperationRecord,
  FileRenameRequest,
} from './fileOperationModel';
import type { RenameNavigation } from './renameNavigation';
import { useDraftStore } from './draftContext';

/** 一份回执最多自动请求一次快照；以后仅由明确按钮再次读取。 */
export function useRenameCompletion(
  controller: EditorController,
  record: FileOperationRecord | null,
  onRenamed?: RenameNavigation,
) {
  const drafts = useDraftStore();
  const lifetime = useRef(new AbortController());
  const started = useRef<{
    request: FileRenameRequest;
    snapshot: EditorSnapshot;
  } | null>(null);
  const attempted = useRef<unknown>(null);
  const reading = useRef(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);
  const readTarget = useCallback(async () => {
    if (
      reading.current ||
      record?.phase !== 'committed' ||
      !record.receipt ||
      record.request.kind !== 'rename'
    )
      return;
    if (!onRenamed) {
      setMessage(
        '回执已保留；当前入口未提供目标重新读取，请从工作台刷新后打开目标。',
      );
      return;
    }
    // 重新打开源页后只允许用户明确选择导航；不重新 POST，也不自动借另一编辑会话的审阅。
    let frozen = started.current;
    if (!frozen) {
      const base = controller.getSnapshot().base;
      if (
        !base ||
        base.path !== record.request.from ||
        base.vault_uuid !== record.request.vault_uuid ||
        base.revision !== record.request.expected_revision ||
        !controller.beginFileOperation(base.revision)
      ) {
        setMessage(
          '这份回执来自另一编辑会话，源版本已变化或有草稿；请保留回执并从工作台另行核对目标。',
        );
        return;
      }
      frozen = { request: record.request, snapshot: controller.getSnapshot() };
      started.current = frozen;
    }
    if (JSON.stringify(record.request) !== JSON.stringify(frozen.request))
      return;
    const original = frozen;
    const isCurrent = () => {
      const base = original.snapshot.base;
      return (
        !lifetime.current.signal.aborted &&
        controller.getSnapshot() === original.snapshot &&
        Boolean(
          base &&
          !drafts.get(base) &&
          !original.snapshot.pending &&
          original.snapshot.phase === 'file-operation',
        )
      );
    };
    if (!isCurrent()) {
      setMessage(
        '源编辑状态已变化，不能用旧审阅跳转；回执已保留，请另行核对。',
      );
      return;
    }
    const signal = lifetime.current.signal;
    reading.current = true;
    setBusy(true);
    setMessage('重命名已提交，正在核对完整快照后重新读取目标…');
    try {
      const opened = await onRenamed(
        frozen.request,
        record.receipt,
        signal,
        isCurrent,
      );
      if (!signal.aborted && !opened)
        setMessage(
          '回执已保留；尚未取得追平本次提交的完整新鲜快照，或源编辑状态已变化。可明确重新读取目标。',
        );
    } catch {
      if (!signal.aborted)
        setMessage('回执已保留，快照核验失败。请稍后重新读取目标。');
    } finally {
      reading.current = false;
      if (!signal.aborted) setBusy(false);
    }
  }, [record, controller, drafts, onRenamed]);
  useEffect(() => {
    if (
      record?.phase === 'committed' &&
      record.receipt &&
      started.current &&
      attempted.current !== record.receipt
    ) {
      attempted.current = record.receipt;
      void readTarget();
    }
  }, [record, readTarget]);
  return {
    busy,
    message,
    readTarget,
    arm(request: FileRenameRequest, snapshot: EditorSnapshot) {
      started.current = { request, snapshot };
      attempted.current = null;
      setMessage('');
      return lifetime.current.signal;
    },
  };
}
