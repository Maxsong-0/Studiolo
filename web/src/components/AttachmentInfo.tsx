import { useLayoutEffect, useRef } from 'react';
import type { TreeEntry } from '../treeTypes';
import { Surface } from '../ui';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';
import '../styles/tree.css';

/**
 * 附件信息浮层（2px 墨框、硬阴影）：只读元数据排成图纸标题栏式的小表——
 * 等宽小标签＋等宽值，格子之间 1px 墨线。不读附件内容，不提供预览或下载。
 */
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

  // Esc 走 dialog 的 cancel：拦下浏览器的直接关闭，改走可中断的收起并归还焦点。
  // 用原生监听而不是 onCancel 属性：React 会让 cancel 沿组件树冒泡，内层对话框的 cancel 也会触发外层的 onCancel；
  // 对话框的原生 cancel 不冒泡，内层的到不了这里。在布局阶段紧跟 showModal 挂上，不留空档。
  useLayoutEffect(() => {
    const element = dialog.current;
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
      ref={dialog}
      appear={false}
      className="attachment-dialog"
      aria-labelledby="attachment-title"
    >
      <header className="attachment-header">
        <div className="attachment-title-group">
          <FileTypeIcon entry={entry} badge />
          <h2 id="attachment-title">附件信息</h2>
        </div>
        <button
          type="button"
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
        <div className="attachment-detail attachment-detail--wide">
          <dt>完整路径</dt>
          <dd>{entry.path}</dd>
        </div>
        <div className="attachment-detail">
          <dt>文件后缀</dt>
          <dd>{extension ? `.${extension}` : '无后缀'}</dd>
        </div>
        <div className="attachment-detail">
          <dt>精确大小</dt>
          <dd>{entry.size_bytes} 字节</dd>
        </div>
      </dl>
      <p className="attachment-note">
        只读元数据 · 大小来自当前快照。后缀不代表已验证的文件类型。
        当前不读取附件内容，不提供预览或下载。
      </p>
    </Surface>
  );
}
