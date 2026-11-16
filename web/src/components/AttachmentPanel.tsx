import { useLayoutEffect, useRef, useState } from 'react';
import type { TreeEntry } from '../treeTypes';
import { Button, Surface } from '../ui';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';
import '../styles/tree.css';
import '../styles/attachment.css';
import { attachmentSize } from '../attachmentApi';
import { useAttachment } from '../useAttachment';

/** 附件预览与下载共用认证读取；可执行类型只生成惰性下载。 */
export function AttachmentPanel({
  entry,
  generation,
  onClose,
}: {
  entry: TreeEntry;
  generation: number;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const download = useRef<HTMLAnchorElement>(null);
  const attachment = useAttachment(entry, generation);
  const { cancel: cancelRead } = attachment;
  const [fullSize, setFullSize] = useState(false);
  const allowed = attachmentSize(entry) !== null;
  const close = () => {
    cancelRead();
    dismiss();
  };
  useLayoutEffect(() => {
    if (
      attachment.url &&
      attachment.requested &&
      document.activeElement === document.body
    )
      download.current?.focus();
  }, [attachment.url, attachment.requested]);
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
      cancelRead();
      dismiss();
    }
    element.addEventListener('cancel', cancel);
    return () => element.removeEventListener('cancel', cancel);
  }, [dismiss, cancelRead]);

  return (
    <Surface
      as="dialog"
      ref={dialog}
      appear={false}
      className="attachment-dialog attachment-panel"
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
          onClick={close}
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
        大小来自当前快照；内容为读取时刻的文件。单文件上限 32 MiB，文本预览上限
        1 MiB。
      </p>
      <div className="attachment-content" aria-live="polite">
        {!allowed ? <p>附件超出网页读取上限，请在同步副本中打开。</p> : null}
        {attachment.loading ? <p role="status">正在读取附件…</p> : null}
        {attachment.error ? <p role="alert">{attachment.error}</p> : null}
        {allowed && !attachment.loading && !attachment.url ? (
          <Button onClick={attachment.load}>
            {attachment.error ? '重试附件' : '准备下载'}
          </Button>
        ) : null}
        {attachment.value && attachment.url ? (
          <>
            <p>类型：{attachment.value.label}</p>
            <div className="attachment-actions">
              <a
                ref={download}
                className="ui-button ui-button--secondary ui-button--md"
                href={attachment.url}
                download={entry.name}
              >
                下载附件
              </a>
              {attachment.value.kind === 'image' ? (
                <Button
                  aria-pressed={fullSize}
                  onClick={() => setFullSize((value) => !value)}
                >
                  {fullSize ? '适应宽度' : '原始尺寸'}
                </Button>
              ) : null}
            </div>
            {attachment.value.kind === 'image' ? (
              <div
                className={`attachment-image${fullSize ? ' attachment-image--original' : ''}`}
              >
                <img src={attachment.url} alt={entry.name} />
              </div>
            ) : null}
            {attachment.value.kind === 'text' ? (
              <pre className="attachment-text">{attachment.value.text}</pre>
            ) : null}
            {attachment.value.kind === 'pdf' ? (
              <p>PDF 可下载后使用系统阅读器打开。</p>
            ) : null}
          </>
        ) : null}
      </div>
    </Surface>
  );
}
