import { useCallback, useState } from 'react';
import { readReplay } from './executionApi';
import { stamp } from './executionView';
import { useExecutionResource } from './useExecutionResource';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  Skeleton,
} from './ui';
import './styles/execution.css';

/** 任务审计回放：等宽时间在左的逐条清单，按服务器给的 offset 翻页。 */
export function ExecutionReplay({
  taskId,
  project,
  revision,
}: {
  taskId: string;
  project: string;
  revision: number;
}) {
  const [after, setAfter] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const load = useCallback(
    (signal: AbortSignal) => readReplay(taskId, project, after, signal),
    [taskId, project, after],
  );
  const page = useExecutionResource(load, revision + reload, false);
  return (
    <Cell className="execution-replay" aria-label="任务审计回放">
      <CellHeader
        title="审计回放"
        level={3}
        meta={page.data ? `本页 ${page.data.events.length} 条` : undefined}
        action={
          <Button
            size="sm"
            onClick={() => {
              setAfter(null);
              setReload((n) => n + 1);
            }}
          >
            重新读取首页
          </Button>
        }
      />
      <p className="execution-hint">
        每页最多 100 条，仅含服务器提供的脱敏元数据；不展示原始调用或正文。
        {after ? `当前页在 offset ${after} 之后。` : '当前为首页。'}
      </p>
      {page.error ? (
        <ErrorState title={page.error} />
      ) : page.loading ? (
        <Skeleton label="正在读取审计记录…" />
      ) : page.data ? (
        <>
          <p className="execution-replay__snapshot">
            回放快照：{page.data.task.title} · {page.data.task.status}
          </p>
          {page.data.events.length ? (
            <ol className="execution-events">
              {page.data.events.map((event) => {
                const time = stamp(event.at);
                return (
                  <li key={event.offset} className="execution-event">
                    {time ? (
                      <time
                        className="execution-event__time"
                        dateTime={event.at}
                      >
                        {time.text}
                      </time>
                    ) : (
                      <span className="execution-event__time">—</span>
                    )}
                    <span className="execution-event__action">
                      {event.action}
                    </span>
                    <span className="execution-event__phase">
                      {event.phase === 'intent' ? '操作意图' : '操作结果'}
                    </span>
                    <span className="execution-event__offset">
                      offset {event.offset}
                      {event.attempt_id ? ` · ${event.attempt_id}` : ''}
                    </span>
                    {Object.keys(event.details).length ? (
                      <dl className="execution-event__details">
                        {Object.entries(event.details).map(([key, value]) => (
                          <div key={key}>
                            <dt>{key}</dt>
                            <dd>{String(value)}</dd>
                          </div>
                        ))}
                      </dl>
                    ) : null}
                  </li>
                );
              })}
            </ol>
          ) : (
            <EmptyState title="此页没有审计事件。" />
          )}
          {page.data.next ? (
            <div className="execution-actions">
              <Button size="sm" onClick={() => setAfter(page.data!.next)}>
                下一页
              </Button>
            </div>
          ) : (
            <p className="execution-hint">已到当前回放末尾。</p>
          )}
        </>
      ) : null}
    </Cell>
  );
}
