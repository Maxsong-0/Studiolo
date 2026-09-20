import { useCallback, useState } from 'react';
import { readReplay } from './executionApi';
import { useExecutionResource } from './useExecutionResource';

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
    <section className="execution-replay" aria-label="任务审计回放">
      <div className="board-toolbar">
        <h3>审计回放</h3>
        <button
          className="outline-button"
          onClick={() => {
            setAfter(null);
            setReload((n) => n + 1);
          }}
        >
          重新读取首页
        </button>
      </div>
      <p>
        每页最多 100 条，仅含服务器提供的脱敏元数据；不展示原始调用或正文。
        {after ? `当前页在 offset ${after} 之后。` : '当前为首页。'}
      </p>
      {page.error ? (
        <p role="alert">{page.error}</p>
      ) : page.loading ? (
        <p role="status">正在读取审计记录…</p>
      ) : page.data ? (
        <>
          <p>
            回放快照：{page.data.task.title} · {page.data.task.status}
          </p>
          {page.data.events.length ? (
            <ol className="execution-events">
              {page.data.events.map((event) => (
                <li key={event.offset}>
                  <p>
                    <time dateTime={event.at}>
                      {new Date(event.at).toLocaleString()}
                    </time>{' '}
                    · {event.action} ·{' '}
                    {event.phase === 'intent' ? '操作意图' : '操作结果'}
                  </p>
                  <p>
                    offset {event.offset}
                    {event.attempt_id ? ` · ${event.attempt_id}` : ''}
                  </p>
                  {Object.keys(event.details).length ? (
                    <dl>
                      {Object.entries(event.details).map(([key, value]) => (
                        <div key={key}>
                          <dt>{key}</dt>
                          <dd>{String(value)}</dd>
                        </div>
                      ))}
                    </dl>
                  ) : null}
                </li>
              ))}
            </ol>
          ) : (
            <p>此页没有审计事件。</p>
          )}
          {page.data.next ? (
            <button
              className="outline-button"
              onClick={() => setAfter(page.data!.next)}
            >
              下一页
            </button>
          ) : (
            <p>已到当前回放末尾。</p>
          )}
        </>
      ) : null}
    </section>
  );
}
