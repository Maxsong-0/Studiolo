import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SourceLocation } from '../types';
import { sourceWindow } from '../sourceWindow';
import { Button, Skeleton, Surface } from '../ui';
import { Icon } from './Icon';
import { DocumentNavigation } from './DocumentNavigation';
import { useDocumentHistory } from '../useDocumentHistory';
import { useSourceDocument } from '../useSourceDocument';
import { useDialogSurface } from '../useDialogSurface';
import { useBlame, type LineBlame } from '../useBlame';
import '../styles/document.css';

/**
 * 原文阅读器：全屏覆盖层（2px 墨框、硬阴影，设计规范第 9 节）。
 * 顶部紧凑的标题、路径与关闭；左 256px 导航（大纲、本页链接、反链，各自可折叠）；
 * 右侧原文按纯文本逐行显示，正文列不超过 72ch。只在同一快照代次内前进后退。
 */
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
  // 逐行归属默认关（T-210）：打开才向服务端要一次。
  const [blameOn, setBlameOn] = useState(false);
  const blame = useBlame(location.path, document?.content ?? null, blameOn);
  const visible = useMemo(
    () => (document ? sourceWindow(document.content, location.line) : null),
    [document, location.line],
  );

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

  useEffect(() => {
    if (document) targetLine.current?.scrollIntoView({ block: 'center' });
  }, [document, location.line]);

  function navigate(next: SourceLocation) {
    if (navigationOpen) navigationToggle.current?.focus();
    setNavigationOpen(false);
    history.navigate(next);
  }

  return (
    <Surface
      as="dialog"
      ref={dialog}
      appear={false}
      className="document-dialog document-reader"
      aria-labelledby="source-title"
      aria-describedby="source-location source-readonly"
    >
      <header className="document-header">
        <div className="document-heading">
          <h2 id="source-title">{document?.title ?? '原文'}</h2>
          <p className="source-location" id="source-location">
            {location.path} · 第 {location.line} 行
          </p>
        </div>
        <button
          type="button"
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
          <Button
            size="sm"
            className="document-back"
            onClick={history.back}
            disabled={!history.canGoBack}
            title="仅在当前快照内浏览"
          >
            <Icon name="arrow" />
            后退
          </Button>
          {document ? (
            <Button
              ref={navigationToggle}
              size="sm"
              className="document-navigation-toggle"
              aria-expanded={navigationOpen}
              aria-controls="document-navigation-panel"
              onClick={() => setNavigationOpen((open) => !open)}
            >
              <Icon name="document" />
              {navigationOpen ? '隐藏导航' : '显示导航'}
            </Button>
          ) : null}
        </div>
        <p className="document-readonly" id="source-readonly">
          原文只读 · 内容按纯文本显示
        </p>
        {document ? (
          <Button
            size="sm"
            className="document-blame-toggle"
            aria-pressed={blameOn}
            onClick={() => setBlameOn((on) => !on)}
          >
            {blameOn ? '隐藏每行是谁改的' : '显示每行是谁改的'}
          </Button>
        ) : null}
        {document && onEdit ? (
          <Button
            size="sm"
            variant="primary"
            className="document-edit"
            onClick={() => onEdit(location.path)}
          >
            编辑与预览
          </Button>
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
          {blameOn && document ? (
            <p
              className="document-blame-note"
              role={blame.error ? 'alert' : 'status'}
            >
              {blame.loading
                ? '正在读取每行是谁改的…'
                : blame.error
                  ? blame.error
                  : blame.mismatch
                    ? '这份原文快照和库里当前的正文不是同一版，行号对不上。刷新工作台后再看。'
                    : `人＝人侧会话 · AI＝代理 · Ob＝Obsidian 同步 · ?＝作者没有记录 · 外＝库外改动 · 早＝早于可查历史${blame.moreHistory ? '（只回看最近 50 版）' : ''}`}
              {blame.error ? (
                <Button variant="text" onClick={blame.retry}>
                  重试
                </Button>
              ) : null}
            </p>
          ) : null}
          {error ? (
            <div className="document-feedback" role="alert">
              <h3>{stale ? '快照已更新，请刷新工作台' : '无法读取原文'}</h3>
              <p>{error}</p>
              <Button size="sm" onClick={stale ? onRefresh : retry}>
                {stale ? '刷新工作台' : '重试'}
              </Button>
            </div>
          ) : document ? (
            <div
              className="source-lines-scroll"
              tabIndex={0}
              aria-label="原文内容"
            >
              <ol
                className={`source-lines${blame.marks ? ' source-lines--blame' : ''}`}
              >
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
                    {blame.marks ? (
                      <BlameCell mark={blame.marks.get(line.number)} />
                    ) : null}
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
            <div className="document-feedback">
              <Skeleton label="正在读取原文…" lines={6} />
            </div>
          )}
        </div>
      </div>
    </Surface>
  );
}

/** 行号左边的归属窄带：每段第一行写缩写、可聚焦，整句放在 title 里；同一段后面的行只留线。 */
function BlameCell({ mark }: { mark: LineBlame | undefined }) {
  if (!mark) return <span className="line-blame" aria-hidden="true" />;
  return mark.first ? (
    <span
      className={`line-blame line-blame--${mark.kind}`}
      title={mark.title}
      aria-label={mark.title}
      role="note"
      tabIndex={0}
    >
      {mark.short}
    </span>
  ) : (
    <span
      className={`line-blame line-blame--${mark.kind}`}
      title={mark.title}
      aria-hidden="true"
    />
  );
}
