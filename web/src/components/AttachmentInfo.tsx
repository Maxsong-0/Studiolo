import { useRef } from 'react';
import type { TreeEntry } from '../treeTypes';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

export function AttachmentInfo({
  entry,
  onClose,
}: {
  entry: TreeEntry;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const dot = entry.name.lastIndexOf('.');
  const extension =
    dot > 0 && dot < entry.name.length - 1 ? entry.name.slice(dot + 1) : null;

  return (
    <dialog
      ref={dialog}
      className="attachment-dialog"
      aria-labelledby="attachment-title"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <header className="attachment-header">
        <div className="attachment-title-group">
          <FileTypeIcon entry={entry} badge />
          <h2 id="attachment-title">附件信息</h2>
        </div>
        <button
          className="icon-button"
          onClick={dismiss}
          aria-label="关闭附件信息"
          data-dialog-dismiss
        >
          <Icon name="close" />
        </button>
      </header>
      <p className="attachment-name">{entry.name}</p>
      <dl className="attachment-details">
        <div>
          <dt>完整路径</dt>
          <dd>{entry.path}</dd>
        </div>
        <div>
          <dt>文件后缀</dt>
          <dd>{extension ? `.${extension}` : '无后缀'}</dd>
        </div>
        <div>
          <dt>精确大小</dt>
          <dd>{entry.size_bytes} 字节</dd>
        </div>
      </dl>
      <p className="attachment-note">
        只读元数据 · 大小来自当前快照。后缀不代表已验证的文件类型。
        当前不读取附件内容，不提供预览或下载。
      </p>
    </dialog>
  );
}
