import { useContext, useLayoutEffect, useRef } from 'react';
import {
  AutoRefreshContext,
  useAutoRefreshReader,
} from '../live/useAutoRefresh';
import { COMMAND_TARGETS, focusCommandTarget } from '../commands/focusTarget';
import { useChanges } from '../useChanges';
import { changeActor, changeLabels } from '../changesModel';
import { useViewLocation } from '../useViewLocation';
import { Button } from '../ui';
import '../styles/changes.css';
/** 只报告首个有界窗口里的未读前三条；没有结果不代表全库没有变化。 */
export function ChangesSummary({
  refreshing = false,
}: {
  refreshing?: boolean;
}) {
  const feed = useChanges(null);
  const { reload, revalidate } = feed;
  useAutoRefreshReader(useContext(AutoRefreshContext), revalidate, 4);
  const wasRefreshing = useRef(refreshing);
  useLayoutEffect(() => {
    if (wasRefreshing.current && !refreshing) void reload();
    wasRefreshing.current = refreshing;
  }, [refreshing, reload]);
  const { goTo } = useViewLocation();
  const recent =
    feed.page?.entries
      .filter((r) => r.kind !== 'gap' && r.unread !== false)
      .slice(0, 3) ?? [];
  return (
    <section className="changes-summary" aria-label="最近内容变化">
      <div className="changes-summary__head">
        <h3>最近内容变化</h3>
        <Button
          variant="text"
          onClick={() => {
            goTo({ view: 'documents', panel: 'changes' });
            focusCommandTarget(COMMAND_TARGETS.changesProject);
          }}
        >
          查看变更流
        </Button>
      </div>
      {feed.loading ? (
        <p role="status">变更读取中…</p>
      ) : feed.error ? (
        <p>
          {feed.unavailable ? '内容变更流尚未启用。' : '内容变更暂不可用。'}
        </p>
      ) : (
        <>
          <ul>
            {recent.map((row) => (
              <li key={row.id}>
                <span
                  aria-label={row.unread === true ? '未读' : '未读状态未知'}
                >
                  {row.unread === true ? '●' : '?'}
                </span>
                <div>
                  <strong>
                    {changeLabels[row.kind]} · {row.path}
                  </strong>
                  <small>{changeActor(row)}</small>
                </div>
              </li>
            ))}
          </ul>
          {recent.length === 0 ? (
            <p>当前窗口没有可列出的未读内容变化。</p>
          ) : null}
          {feed.page?.next_cursor ||
          feed.page?.history_incomplete ||
          feed.page?.retired ||
          feed.page?.seen_complete === false ||
          feed.page?.entries.some((r) => r.kind === 'gap') ? (
            <p className="changes-caption">
              记录或已读水位可能不完整，打开变更流查看。
            </p>
          ) : null}
        </>
      )}
    </section>
  );
}
