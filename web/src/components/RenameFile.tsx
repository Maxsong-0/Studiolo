import { useState } from 'react';
import type { EditorController, EditorSnapshot } from '../EditorController';
import type { FileRenameRequest } from '../fileOperationModel';
import type { RenameNavigation } from '../renameNavigation';
import { useFileOperationStore } from '../fileOperationContext';
import { useFileOperations } from '../useFileOperations';
import { renameTargetError } from '../renameValidation';
import { useRenameCompletion } from '../useRenameCompletion';
import { Button, Surface } from '../ui';
import { FileOperationStatus } from './FileOperationStatus';
import '../styles/file-operations.css';

export function RenameFile({
  controller,
  state,
  onRenamed,
}: {
  controller: EditorController;
  state: EditorSnapshot;
  onRenamed?: RenameNavigation;
}) {
  const { store, record } = useFileOperationStore();
  const capability = useFileOperations();
  const [reviewing, setReviewing] = useState(false);
  const [to, setTo] = useState('');
  const [confirmation, setConfirmation] = useState<{
    state: EditorSnapshot;
    to: string;
  } | null>(null);
  const completion = useRenameCompletion(controller, record, onRenamed);
  const base = state.base;
  const matchesRecord =
    record?.request.kind === 'rename' &&
    (record.request.from === base?.path || record.request.to === base?.path) &&
    record.request.vault_uuid === base?.vault_uuid;
  if (!base || (!capability.renameEnabled && !matchesRecord)) return null;
  const targetError = renameTargetError(base.path, to);
  const allowed = Boolean(
    capability.renameEnabled &&
    capability.vault === base.vault_uuid &&
    controller.canRestoreHistory() &&
    store.canStart(),
  );
  const confirmed = confirmation?.state === state && confirmation.to === to;
  const submit = () => {
    if (
      !allowed ||
      !confirmed ||
      targetError ||
      controller.getSnapshot() !== state ||
      !store.canStart()
    )
      return;
    const saved = controller.beginFileOperation(base.revision);
    if (!saved) return;
    const request: FileRenameRequest = Object.freeze({
      kind: 'rename',
      vault_uuid: saved.vault_uuid,
      from: saved.path,
      to,
      expected_revision: saved.revision,
      idempotency_key: crypto.randomUUID(),
    });
    const signal = completion.arm(request, controller.getSnapshot());
    setConfirmation(null);
    void store.submit(request, signal);
  };
  return (
    <section
      className="file-op file-rename"
      data-editor-no-save
      aria-label="重命名文件"
    >
      <Button
        size="sm"
        disabled={!allowed}
        aria-expanded={reviewing}
        onClick={() => {
          setReviewing((value) => !value);
          setConfirmation(null);
        }}
      >
        重命名文件
      </Button>
      {reviewing && !matchesRecord ? (
        <Surface className="file-op__review">
          <p>
            源已保存版本：
            <strong className="file-op__path">{base.path}</strong>
          </p>
          <p className="file-op__revision">版本：{base.revision}</p>
          <label className="rename-target">
            目标完整路径
            <input
              type="text"
              value={to}
              spellCheck={false}
              autoComplete="off"
              disabled={!allowed}
              onChange={(event) => {
                setTo(event.target.value);
                setConfirmation(null);
              }}
            />
          </label>
          <p>
            只支持获批准的 Markdown
            文本。目标父目录须已存在，不覆盖同名文件、不创建父目录；文件内链接不会自动改写。
          </p>
          <p>修改正文、保存、输入法状态或目标路径变化后，都必须重新确认。</p>
          {to && targetError ? <p role="alert">{targetError}</p> : null}
          <label className="file-op__confirm">
            <input
              type="checkbox"
              checked={confirmed && allowed && !targetError}
              disabled={!allowed || Boolean(targetError)}
              onChange={(event) =>
                setConfirmation(event.target.checked ? { state, to } : null)
              }
            />
            我确认重命名这个已保存版本到上述目标
          </label>
          <div className="file-op__actions">
            <Button
              size="sm"
              variant="danger"
              disabled={!allowed || !confirmed || Boolean(targetError)}
              onClick={submit}
            >
              确认重命名
            </Button>
            <Button
              size="sm"
              variant="text"
              onClick={() => {
                setReviewing(false);
                setConfirmation(null);
                setTo('');
              }}
            >
              取消重命名审阅
            </Button>
          </div>
        </Surface>
      ) : null}
      {matchesRecord ? (
        <>
          <FileOperationStatus />
          {record.phase === 'committed' &&
          record.request.kind === 'rename' &&
          record.request.from === base.path ? (
            <section className="file-receipt" aria-label="重新读取重命名目标">
              <p role="status">
                {completion.message ||
                  '回执已保存。目标文档需要独立读取并重新核对权限。'}
              </p>
              <Button
                size="sm"
                aria-disabled={completion.busy}
                onClick={() => {
                  if (!completion.busy) void completion.readTarget();
                }}
              >
                {completion.busy ? '正在核验目标…' : '重新读取目标'}
              </Button>
            </section>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
