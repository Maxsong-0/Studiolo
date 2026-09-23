import type { EditorController, EditorSnapshot } from '../EditorController';
import { EDIT_LIMIT } from '../canonicalModel';
import { usePageHistory } from '../usePageHistory';
import { Button, ErrorState, StatusMark } from '../ui';
import { HistoryMetadata } from './HistoryMetadata';
import { HistoryComparison } from './HistoryComparison';
import { EditorTextBoundary } from './EditorTextBoundary';
import '../styles/history.css';

/**
 * 页面历史（编辑器内的一段）：版本逐行排开，时间与主体用等宽读数；选中的版本黑底反白。
 * 正文超限的版本只列元数据（灰色虚线方块）；比对与恢复见 HistoryComparison。
 */
export function PageHistory({
  controller,
  state,
  onClose,
  onRestoreStart,
}: {
  controller: EditorController;
  state: EditorSnapshot;
  onClose: () => void;
  onRestoreStart: () => void;
}) {
  const base = state.base!;
  const history = usePageHistory(base);
  const allowed =
    controller.canRestoreHistory() && history.file?.revision !== base.revision;
  return (
    <section
      className="page-history"
      aria-labelledby="page-history-title"
      data-editor-no-save
    >
      <header className="history-heading">
        <div>
          <h3 id="page-history-title">页面历史</h3>
          <p>
            页历史不等于灾难备份。列表和回执都不是当前磁盘内容的证明。时间按本机时区显示。
          </p>
        </div>
        <Button size="sm" onClick={onClose}>
          关闭历史
        </Button>
      </header>
      <div className="history-pagination">
        <Button
          size="sm"
          aria-disabled={history.loading}
          onClick={() => {
            if (!history.loading) void history.readPage(null);
          }}
        >
          {history.changed
            ? '历史已变化，重新读取第一页'
            : history.cursor
              ? '返回第一页'
              : '重新读取历史'}
        </Button>
        <Button
          size="sm"
          disabled={history.loading || !history.page?.next_cursor}
          onClick={() => {
            if (history.page?.next_cursor)
              void history.readPage(history.page.next_cursor);
          }}
        >
          下一页
        </Button>
      </div>
      {history.loading ? (
        <p className="history-status" role="status">
          正在读取页面历史…
        </p>
      ) : null}
      {history.error ? <ErrorState title={history.error} /> : null}
      {history.page?.entries.length === 0 ? (
        <p className="history-status">此页暂无可列出的历史版本。</p>
      ) : null}
      <ul className="history-entries">
        {history.page?.entries.map((entry) => (
          <li key={entry.id}>
            <Button
              size="sm"
              className="history-select"
              aria-pressed={history.selected?.id === entry.id}
              disabled={entry.bytes > EDIT_LIMIT}
              onClick={() => void history.select(entry)}
            >
              查看版本 <code>{entry.revision}</code>
            </Button>
            <HistoryMetadata entry={entry} />
            {entry.bytes > EDIT_LIMIT ? (
              <p className="history-oversize">
                <StatusMark state="dormant">
                  正文超过 256 KiB，仅列出元数据，网页不能预览或恢复此版本。
                </StatusMark>
              </p>
            ) : null}
          </li>
        ))}
      </ul>
      {history.reading ? (
        <p className="history-status" role="status">
          正在核对所选历史正文…
        </p>
      ) : null}
      {history.file && history.selected ? (
        <EditorTextBoundary
          key={history.selected.id}
          content={history.file.content}
        >
          <HistoryComparison
            key={`${history.selected.id}:${base.revision}:${allowed}`}
            file={history.file}
            draft={state.draft}
            revision={base.revision}
            allowed={allowed}
            onRestore={() => {
              if (history.file) {
                onRestoreStart();
                void controller.restoreHistory(history.file);
              }
            }}
          />
        </EditorTextBoundary>
      ) : null}
    </section>
  );
}
