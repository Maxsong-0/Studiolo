import { useEffect, useMemo, useRef, useState } from 'react';
import type { SourceLocation } from '../types';
import { sourceWindow } from '../sourceWindow';
import { Icon } from './Icon';
import { DocumentNavigation } from './DocumentNavigation';
import { useDocumentHistory } from '../useDocumentHistory';
import { useSourceDocument } from '../useSourceDocument';
import { useDialogSurface } from '../useDialogSurface';

export function DocumentDialog({
  source,
  generation,
  onClose,
  onRefresh,
  onEdit,
}: {
  source: SourceLocation;
  generation: number;
  onClose: () => void;
  onRefresh: () => void;
  onEdit?: (path: string) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const targetLine = useRef<HTMLLIElement>(null);
  const navigationToggle = useRef<HTMLButtonElement>(null);
  const [navigationOpen, setNavigationOpen] = useState(false);
  const history = useDocumentHistory(source);
  const location = history.location;
  const { document, error, stale, retry } = useSourceDocument(
    location.path,
    generation,
  );
  const visible = useMemo(
    () => (document ? sourceWindow(document.content, location.line) : null),
    [document, location.line],
  );

  useEffect(() => {
    if (document) targetLine.current?.scrollIntoView({ block: 'center' });
  }, [document, location.line]);

  function navigate(next: SourceLocation) {
    if (navigationOpen) navigationToggle.current?.focus();
    setNavigationOpen(false);
    history.navigate(next);
  }

  return (
    <dialog
      ref={dialog}
      className="document-dialog"
      aria-labelledby="source-title"
      aria-describedby="source-location source-readonly"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <header className="document-header">
        <div>
          <h2 id="source-title">{document?.title ?? '原文'}</h2>
          <p className="source-location" id="source-location">
            {location.path} · 第 {location.line} 行
          </p>
        </div>
        <button
          className="icon-button"
          onClick={dismiss}
          data-dialog-dismiss
          aria-label="关闭原文"
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="document-toolbar">
        <div className="document-history">
          <button
            className="document-back"
            onClick={history.back}
            disabled={!history.canGoBack}
            title="仅在当前快照内浏览"
          >
            <Icon name="arrow" />
            后退
          </button>
          {document ? (
            <button
              ref={navigationToggle}
              className="document-navigation-toggle"
              aria-expanded={navigationOpen}
              aria-controls="document-navigation-panel"
              onClick={() => setNavigationOpen((open) => !open)}
            >
              <Icon name="document" />
              {navigationOpen ? '隐藏导航' : '显示导航'}
            </button>
          ) : null}
        </div>
        <p className="document-readonly" id="source-readonly">
          原文只读 · 内容按纯文本显示
        </p>
        {document && onEdit ? (
          <button
            className="outline-button"
            onClick={() => onEdit(location.path)}
          >
            编辑与预览
          </button>
        ) : null}
      </div>
      <div
        className={`document-body${navigationOpen ? ' is-navigation-open' : ''}`}
      >
        {document ? (
          <aside
            id="document-navigation-panel"
            className={`document-navigation-panel${navigationOpen ? ' is-open' : ''}`}
          >
            <DocumentNavigation
              key={document.path}
              navigation={document.navigation}
              onNavigate={navigate}
              currentLine={location.line}
            />
          </aside>
        ) : null}
        <div className="document-content">
          {visible?.large ? (
            <p className="large-document-note" role="status">
              文档较大，仅显示目标附近原文（第 {visible.start}–{visible.end}{' '}
              行）
            </p>
          ) : null}
          {error ? (
            <div className="document-feedback" role="alert">
              <h3>{stale ? '快照已更新，请刷新工作台' : '无法读取原文'}</h3>
              <p>{error}</p>
              <button
                className="outline-button"
                onClick={stale ? onRefresh : retry}
              >
                {stale ? '刷新工作台' : '重试'}
              </button>
            </div>
          ) : document ? (
            <div
              className="source-lines-scroll"
              tabIndex={0}
              aria-label="原文内容"
            >
              <ol className="source-lines">
                {visible?.lines.map((line) => (
                  <li
                    key={line.number}
                    ref={line.number === location.line ? targetLine : undefined}
                    className={
                      line.number === location.line ? 'target-line' : undefined
                    }
                    aria-current={
                      line.number === location.line ? 'location' : undefined
                    }
                  >
                    <span className="line-number" aria-hidden="true">
                      {line.number}
                    </span>
                    <span className="line-content">
                      {line.text || '\u00a0'}
                    </span>
                  </li>
                ))}
              </ol>
            </div>
          ) : (
            <p className="document-feedback" role="status">
              正在读取原文…
            </p>
          )}
        </div>
      </div>
    </dialog>
  );
}
