import { useEffect, useId, useRef, useState } from 'react';
import { queryAgentReads } from '../policyApi';
import { ApiError } from '../sessionHttp';
import { displayTime } from '../policy/labels';
import { Button, Cell, CellHeader } from '../ui';
import { useViewLocation } from '../useViewLocation';
import {
  parseReadAudit,
  type ReadAuditPage,
  type ReadAuditQuery,
  type ReadOutcome,
} from './readModel';

function localTime(ms: number) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}T${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}
type Fields = {
  outcome: ReadOutcome;
  agent_type: string;
  instance_id: string;
  since: string;
  until: string;
  path_prefix: string;
};
type State =
  | { kind: 'idle' | 'loading' }
  | { kind: 'failed'; message: string }
  | { kind: 'ready'; page: ReadAuditPage };
export function ReadAuditPanel({ types }: { types: string[] }) {
  const id = useId(),
    { goTo } = useViewLocation();
  const [fields, setFields] = useState<Fields>(() => ({
    outcome: 'read',
    agent_type: '',
    instance_id: '',
    since: localTime(Date.now() - 86_400_000),
    until: '',
    path_prefix: '',
  }));
  const [state, setState] = useState<State>({ kind: 'idle' });
  const [history, setHistory] = useState<(number | null)[]>([null]);
  const [applied, setApplied] = useState<ReadAuditQuery | null>(null);
  const controller = useRef<AbortController | null>(null);
  useEffect(() => () => controller.current?.abort(), []);
  function edit<K extends keyof Fields>(key: K, value: Fields[K]) {
    controller.current?.abort();
    setFields((f) => ({
      ...f,
      [key]: value,
      ...(key === 'outcome' ? { path_prefix: '' } : {}),
    }));
    setState({ kind: 'idle' });
    setHistory([null]);
    setApplied(null);
  }
  async function load(query: ReadAuditQuery, pages: (number | null)[]) {
    controller.current?.abort();
    const current = new AbortController();
    controller.current = current;
    setState({ kind: 'loading' });
    try {
      const page = parseReadAudit(
        await queryAgentReads(query, current.signal),
        query.outcome,
        query.before,
      );
      if (!current.signal.aborted) {
        setState({ kind: 'ready', page });
        const before = query.before ?? page.head_offset + 1;
        setHistory(pages.map((value) => value ?? before));
        setApplied({ ...query, before });
      }
    } catch (error) {
      if (!current.signal.aborted)
        setState({
          kind: 'failed',
          message:
            error instanceof ApiError && error.status === 404
              ? '本入口尚未接入读取审计。'
              : error instanceof ApiError && error.status === 403
                ? '需要有效的主人会话才能查询审计。'
                : '审计账本暂不可用或响应无法验证，请重新查询。',
        });
    }
  }
  function submit() {
    let since: string | null, until: string;
    try {
      since = fields.since ? new Date(fields.since).toISOString() : null;
      until = fields.until
        ? new Date(fields.until).toISOString()
        : new Date().toISOString();
    } catch {
      setState({ kind: 'failed', message: '请输入有效的开始与结束时间。' });
      return;
    }
    if (since && Date.parse(since) > Date.parse(until)) {
      setState({ kind: 'failed', message: '开始时间不能晚于结束时间。' });
      return;
    }
    void load(
      {
        outcome: fields.outcome,
        agent_type: fields.agent_type || null,
        instance_id: fields.instance_id || null,
        since,
        until,
        path_prefix:
          fields.outcome === 'read' ? fields.path_prefix || null : null,
        before: null,
      },
      [null],
    );
  }
  return (
    <Cell span={12} className="policy-cell read-audit">
      <CellHeader title="哪个 AI 读了我什么" />
      <p className="policy-note">
        来自已验链的权威账本；每页检查最多 256 条记录。筛选条件只用于本次查询。
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        className="read-audit__form"
      >
        <label htmlFor={`${id}-outcome`}>
          记录类别
          <select
            className="policy-input"
            id={`${id}-outcome`}
            value={fields.outcome}
            onChange={(e) => edit('outcome', e.target.value as ReadOutcome)}
          >
            <option value="read">成功读取</option>
            <option value="denied">被拒绝的动作</option>
          </select>
        </label>
        <label htmlFor={`${id}-type`}>
          AI 类型
          <select
            className="policy-input"
            id={`${id}-type`}
            value={fields.agent_type}
            onChange={(e) => edit('agent_type', e.target.value)}
          >
            <option value="">全部类型（含未认证）</option>
            {types.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor={`${id}-instance`}>
          实例号
          <input
            className="policy-input"
            id={`${id}-instance`}
            value={fields.instance_id}
            maxLength={26}
            pattern="[A-Za-z0-9]{26}"
            onChange={(e) => edit('instance_id', e.target.value)}
            placeholder="留空查询全部实例"
          />
        </label>
        <label htmlFor={`${id}-since`}>
          开始时间（本地）
          <input
            className="policy-input"
            id={`${id}-since`}
            type="datetime-local"
            value={fields.since}
            onChange={(e) => edit('since', e.target.value)}
          />
        </label>
        <label htmlFor={`${id}-until`}>
          结束时间（本地）
          <input
            className="policy-input"
            id={`${id}-until`}
            type="datetime-local"
            value={fields.until}
            onChange={(e) => edit('until', e.target.value)}
          />
        </label>
        <label htmlFor={`${id}-path`}>
          路径前缀
          <input
            className="policy-input"
            id={`${id}-path`}
            disabled={fields.outcome === 'denied'}
            value={fields.path_prefix}
            maxLength={1024}
            onChange={(e) => edit('path_prefix', e.target.value)}
            placeholder="留空查询全部路径"
          />
        </label>
        <div className="read-audit__actions">
          <Button type="submit" disabled={state.kind === 'loading'}>
            查询审计
          </Button>
          {state.kind === 'loading' ? (
            <Button
              variant="secondary"
              onClick={() => {
                controller.current?.abort();
                setState({ kind: 'idle' });
              }}
            >
              取消查询
            </Button>
          ) : null}
        </div>
      </form>
      {fields.outcome === 'denied' ? (
        <p className="policy-hint">
          拒绝记录只保存动作与原因，没有内容路径；未认证的调用不会推断所属 AI。
        </p>
      ) : null}
      <div aria-live="polite">
        {state.kind === 'loading' ? (
          <p>正在验证并查询审计…</p>
        ) : state.kind === 'failed' ? (
          <p role="alert">{state.message}</p>
        ) : state.kind === 'ready' ? (
          <>
            <p>
              本页检查 {state.page.scanned} 条账本记录，匹配{' '}
              {state.page.records.length} 条。
              {state.page.next_before !== null
                ? '还有更早记录可查询。'
                : '已到记录起点。'}
            </p>
            {state.page.records.length === 0 ? (
              <p>
                本页没有匹配记录。
                {state.page.next_before !== null
                  ? '继续查询更早记录，不能据此判断全库没有匹配。'
                  : ''}
              </p>
            ) : null}
            <ol className="read-audit__records">
              {state.page.records.map((row) => (
                <li key={row.offset}>
                  <article aria-label={`审计记录 ${row.offset}`}>
                    <header>
                      <time dateTime={row.recorded_at}>
                        {displayTime(row.recorded_at)}
                      </time>
                      <span>{row.agent_type ?? '未认证调用'}</span>
                    </header>
                    <dl>
                      <div>
                        <dt>实例</dt>
                        <dd>
                          {row.instance_id ? (
                            <button
                              type="button"
                              className="textButton"
                              onClick={() =>
                                goTo({
                                  view: 'policy',
                                  panel: 'agents',
                                  item: row.instance_id!,
                                })
                              }
                            >
                              {row.instance_id}
                            </button>
                          ) : (
                            '未登记'
                          )}
                        </dd>
                      </div>
                      <div>
                        <dt>动作</dt>
                        <dd>
                          <code>{row.action}</code>
                        </dd>
                      </div>
                      {row.path !== null ? (
                        <>
                          <div>
                            <dt>路径</dt>
                            <dd>{row.path}</dd>
                          </div>
                          <div>
                            <dt>供给版本</dt>
                            <dd>
                              <code>{row.revision}</code>
                            </dd>
                          </div>
                        </>
                      ) : (
                        <div>
                          <dt>拒绝原因</dt>
                          <dd>
                            <code>{row.reason}</code>
                          </dd>
                        </div>
                      )}
                      <div>
                        <dt>账本偏移</dt>
                        <dd>{row.offset}</dd>
                      </div>
                    </dl>
                  </article>
                </li>
              ))}
            </ol>
            <div className="read-audit__actions">
              <Button
                variant="secondary"
                disabled={history.length < 2}
                onClick={() => {
                  if (applied)
                    void load(
                      { ...applied, before: history[history.length - 2] },
                      history.slice(0, -1),
                    );
                }}
              >
                上一页
              </Button>
              <Button
                variant="secondary"
                disabled={state.page.next_before === null}
                onClick={() => {
                  if (applied && state.page.next_before !== null)
                    void load({ ...applied, before: state.page.next_before }, [
                      ...history,
                      state.page.next_before,
                    ]);
                }}
              >
                更早记录
              </Button>
            </div>
          </>
        ) : (
          <p>设定筛选条件后查询，默认查看最近 24 小时。</p>
        )}
      </div>
    </Cell>
  );
}
