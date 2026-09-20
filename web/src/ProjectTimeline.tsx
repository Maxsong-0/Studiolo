import { useCallback, useState } from 'react';
import { getJson } from './sessionHttp';
import { useExecutionResource } from './useExecutionResource';

import { timelinePage } from './projectTimelineModel';

export function ProjectTimeline({ project }: { project: string }) {
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
    <section aria-label="项目时间回放" className="board-item">
      <div className="board-toolbar">
        <label>
          时间范围{' '}
          <select
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
        <button
          onClick={() => {
            setAfter(null);
            setRevision((n) => n + 1);
          }}
        >
          重新核对回放
        </button>
      </div>
      <label>
        回放至 {new Date(cutoff).toLocaleString()}
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
      {page.error ? (
        <p role="alert">{page.error}</p>
      ) : page.loading ? (
        <p role="status">正在读取项目回放…</p>
      ) : (
        <>
          {!events.length && !logs.length ? (
            <p>
              {page.data?.next
                ? '本页尚无匹配活动，请继续下一页。'
                : '所选范围无活动。'}
            </p>
          ) : null}
          <ol aria-label="黑板历史事件">
            {events.map((e) => (
              <li key={e.offset}>
                <time dateTime={e.at}>{new Date(e.at).toLocaleString()}</time> ·{' '}
                {e.action} · {e.phase === 'intent' ? '意图' : '结果'}
                <br />
                <code>{e.task_id}</code>
              </li>
            ))}
          </ol>
          <h3>项目日志</h3>
          <ul>
            {logs.map((l) => (
              <li key={l.path}>
                {l.date} · {l.title}
                <br />
                <code>{l.path}</code>
              </li>
            ))}
          </ul>
          {page.data?.undated_logs ? (
            <p>
              另有 {page.data.undated_logs}{' '}
              篇日志无法从文件名确定日期，未猜测其时间。
            </p>
          ) : null}
          {page.data?.next ? (
            <button onClick={() => setAfter(page.data!.next)}>
              下一页事件
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}
