import { useCallback, useState } from 'react';
import type { ReactNode } from 'react';
import { getJson } from './sessionHttp';
import { useExecutionResource } from './useExecutionResource';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  Skeleton,
  StatusMark,
} from './ui';
import { timelinePage } from './projectTimelineModel';
import './styles/board.css';
import './styles/collab.css';

export function ProjectTimeline({
  project,
  leading,
}: {
  project: string;
  /** 工具行最前面的控件（协作页放项目选择）。 */
  leading?: ReactNode;
}) {
  const [end] = useState(() => new Date().toISOString());
  const [days, setDays] = useState(1);
  const [position, setPosition] = useState(1000);
  const [after, setAfter] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const from = new Date(Date.parse(end) - days * 86400_000).toISOString();
  const load = useCallback(
    async (signal: AbortSignal) => {
      const params = new URLSearchParams({ project, from, to: end });
      if (after) params.set('after', after);
      const result = timelinePage(
        await getJson(`/api/v1/board/timeline?${params}`, signal),
        project,
        from,
        end,
      );
      if (
        after !== null &&
        ((result.next !== null && BigInt(result.next) <= BigInt(after)) ||
          result.events.some((e) => BigInt(e.offset) <= BigInt(after)))
      )
        throw new Error('回放游标未前进。');
      return result;
    },
    [project, from, end, after],
  );
  const page = useExecutionResource(load, revision, false);
  const cutoff =
    Date.parse(from) + ((Date.parse(end) - Date.parse(from)) * position) / 1000;
  const events =
    page.data?.events.filter((e) => Date.parse(e.at) <= cutoff) ?? [];
  const logs =
    page.data?.logs.filter(
      (l) => Date.parse(`${l.date}T00:00:00Z`) <= cutoff,
    ) ?? [];
  return (
    <section aria-label="项目时间回放" className="timeline">
      <GridFrame className="timeline__grid">
        <Cell className="timeline__controls">
          <div className="board-toolbar">
            {leading}
            <label className="collab-select">
              时间范围{' '}
              <select
                className="collab-select__control"
                value={days}
                onChange={(e) => {
                  setDays(Number(e.target.value));
                  setAfter(null);
                }}
              >
                <option value={1}>最近 24 小时</option>
                <option value={7}>最近 7 天</option>
                <option value={30}>最近 30 天</option>
              </select>
            </label>
            <Button
              size="sm"
              onClick={() => {
                setAfter(null);
                setRevision((n) => n + 1);
              }}
            >
              重新核对回放
            </Button>
          </div>
          <label className="timeline__scrub">
            <span className="timeline__scrub-label">
              回放至{' '}
              <time dateTime={new Date(cutoff).toISOString()}>
                {new Date(cutoff).toLocaleString()}
              </time>
            </span>
            <input
              aria-label="回放时间轴"
              type="range"
              min={0}
              max={1000}
              value={position}
              onChange={(e) => setPosition(Number(e.target.value))}
            />
          </label>
          <p className="board-hint">
            展示该时间之前的事件与当日记录；日志按 UTC
            日期归组，内容以当前文件为准，不将当前工作项状态当作历史快照。
          </p>
        </Cell>
        {page.error ? (
          <Cell>
            <ErrorState title={page.error} />
          </Cell>
        ) : page.loading ? (
          <Cell>
            <Skeleton label="正在读取项目回放…" />
          </Cell>
        ) : (
          <>
            <Cell span={8} className="timeline__events">
              <CellHeader
                title="黑板历史事件"
                level={3}
                meta={`${events.length} 条`}
              />
              {!events.length && !logs.length ? (
                <EmptyState
                  title={
                    page.data?.next
                      ? '本页尚无匹配活动，请继续下一页。'
                      : '所选范围无活动。'
                  }
                />
              ) : null}
              <ol className="timeline__list" aria-label="黑板历史事件">
                {events.map((e) => (
                  <li key={e.offset} className="timeline__event">
                    <time className="timeline__time" dateTime={e.at}>
                      {new Date(e.at).toLocaleString()}
                    </time>
                    <span className="timeline__action">{e.action}</span>
                    <StatusMark
                      className="timeline__phase"
                      state={e.phase === 'intent' ? 'pending' : 'ok'}
                    >
                      {e.phase === 'intent' ? '意图' : '结果'}
                    </StatusMark>
                    <code className="timeline__task">{e.task_id}</code>
                  </li>
                ))}
              </ol>
              {page.data?.next ? (
                <div className="timeline__more">
                  <Button size="sm" onClick={() => setAfter(page.data!.next)}>
                    下一页事件
                  </Button>
                </div>
              ) : null}
            </Cell>
            <Cell span={4} className="timeline__logs">
              <CellHeader
                title="项目日志"
                level={3}
                meta={`${logs.length} 篇`}
              />
              <ul className="timeline__list">
                {logs.map((l) => (
                  <li key={l.path} className="timeline__log">
                    <time className="timeline__time" dateTime={l.date}>
                      {l.date}
                    </time>
                    <span className="timeline__log-title">{l.title}</span>
                    <code className="timeline__log-path">{l.path}</code>
                  </li>
                ))}
              </ul>
              {page.data?.undated_logs ? (
                <p className="board-hint">
                  另有 {page.data.undated_logs}{' '}
                  篇日志无法从文件名确定日期，未猜测其时间。
                </p>
              ) : null}
            </Cell>
          </>
        )}
      </GridFrame>
    </section>
  );
}
