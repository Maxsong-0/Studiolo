import type { OpenSource } from '../types';
import type { useSearch } from '../useSearch';
import { Icon } from './Icon';

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
      <p className="search-feedback" role="status">
        正在搜索当前快照…
      </p>
    );
  if (search.error)
    return (
      <div className="search-feedback" role="alert">
        <p>
          {search.status === 409
            ? '快照已更新，请刷新工作台。'
            : search.status === 429
              ? '搜索服务正忙，请稍后重试。'
              : ''}
          {search.error}
        </p>
        {!search.invalid ? (
          <button
            className="text-button"
            onClick={search.status === 409 ? onRefresh : search.retry}
          >
            {search.status === 409 ? '刷新工作台' : '重试搜索'}
          </button>
        ) : null}
      </div>
    );
  if (!search.result?.hits.length)
    return (
      <p className="search-feedback" role="status">
        {emptyMessage}
      </p>
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
              data-search-result
              className="query-result-open"
              onClick={() => onOpen({ path: hit.path, line: hit.line })}
              aria-label={`${hit.title}，查看原文`}
            >
              <Icon name="document" />
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
                <Icon name="chevron" />
              </span>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}
