import type { OpenSource } from '../types';
import type { useSearch } from '../useSearch';
import { EmptyState, ErrorState, Skeleton } from '../ui';
import '../styles/search.css';

/**
 * 搜索结果（格子内的行）：标题 15.5/700、等宽路径与行号、片段只作纯文本，右侧「查看原文」。
 * 整行是一个按钮，读屏名字是「标题，查看原文」；右侧的按钮外观只是视觉提示。
 */
export function QueryResults({
  search,
  onOpen,
  onRefresh,
  emptyMessage,
}: {
  search: ReturnType<typeof useSearch>;
  onOpen: OpenSource;
  onRefresh: () => void;
  emptyMessage: string;
}) {
  if (search.loading)
    return (
      <div className="search-feedback">
        <Skeleton label="正在搜索当前快照…" lines={3} />
      </div>
    );
  if (search.error) {
    const notice =
      search.status === 409
        ? '快照已更新，请刷新工作台。'
        : search.status === 429
          ? '搜索服务正忙，请稍后重试。'
          : null;
    return (
      <div className="search-feedback">
        <ErrorState
          title={notice ?? search.error}
          onRetry={
            search.invalid
              ? undefined
              : search.status === 409
                ? onRefresh
                : search.retry
          }
          retryLabel={search.status === 409 ? '刷新工作台' : '重试搜索'}
        >
          {notice ? <p>{search.error}</p> : null}
        </ErrorState>
      </div>
    );
  }
  if (!search.result?.hits.length)
    return (
      <div className="search-feedback" role="status">
        <EmptyState title={emptyMessage} />
      </div>
    );
  return (
    <>
      {search.result.truncated ? (
        <p className="search-limit" role="status">
          结果较多，仅显示前 100 条；请缩短范围或补充搜索词。
        </p>
      ) : null}
      <ul className="query-results" aria-label="搜索结果">
        {search.result.hits.map((hit, index) => (
          <li key={`${hit.path}:${hit.line}:${hit.kind}:${index}`}>
            <button
              type="button"
              data-search-result
              className="query-result-open"
              onClick={() => onOpen({ path: hit.path, line: hit.line })}
              aria-label={`${hit.title}，查看原文`}
            >
              <span className="query-result-content">
                <span className="document-title">{hit.title}</span>
                <span className="document-path">
                  {hit.path} · 第 {hit.line} 行
                </span>
                {hit.snippet ? (
                  <span className="query-snippet">{hit.snippet}</span>
                ) : null}
              </span>
              <span className="query-open-hint" aria-hidden="true">
                查看原文
                <span className="query-open-arrow">→</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}
