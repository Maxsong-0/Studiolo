import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { useFileOperationStore } from './fileOperationContext';
import type { Workbench } from './types';
import type { RenameNavigation } from './renameNavigation';
import { renameSnapshotReady } from './renameNavigation';
import { renameReceipt } from './renameValidation';

interface OpenEditor {
  path: string;
  id: number;
  scope: string | null;
  vault?: string;
}
/** 原 dialog 的身份独立于路径；关闭后重开同路径也不能接旧请求的迟到导航。 */
export function useEditorNavigation(
  refreshLatest: () => Promise<Workbench | null>,
) {
  const { store } = useFileOperationStore();
  const [selected, setSelected] = useState<OpenEditor | null>(null);
  const active = useRef<OpenEditor | null>(null);
  const serial = useRef(0);
  useLayoutEffect(
    () => () => {
      active.current = null;
    },
    [store.ownerScope],
  );
  const open = useCallback(
    (path: string | null) => {
      const next =
        path === null
          ? null
          : { path, id: ++serial.current, scope: store.ownerScope };
      active.current = next;
      setSelected(next);
    },
    [store.ownerScope],
  );
  const onRenamed = useCallback<RenameNavigation>(
    async (request, receipt, signal, editorCurrent) => {
      const origin = active.current;
      const record = store.getSnapshot();
      const current = () =>
        Boolean(
          origin &&
          origin === active.current &&
          origin.scope === store.ownerScope &&
          store.ownerScope &&
          origin.path === request.from &&
          !signal.aborted &&
          editorCurrent() &&
          store.getSnapshot() === record &&
          record?.phase === 'committed' &&
          record.receipt === receipt &&
          JSON.stringify(record.request) === JSON.stringify(request),
        );
      if (!current()) return false;
      try {
        renameReceipt({ status: 'committed', receipt }, request);
        const snapshot = await refreshLatest();
        if (!current() || !renameSnapshotReady(snapshot, receipt.sequence))
          return false;
        const target = {
          path: request.to,
          id: ++serial.current,
          scope: store.ownerScope,
          vault: request.vault_uuid,
        };
        active.current = target;
        setSelected(target);
        return true;
      } catch {
        return false;
      }
    },
    [store, refreshLatest],
  );
  return {
    current: selected?.scope === store.ownerScope ? selected : null,
    open,
    onRenamed,
  };
}
