import { useCallback, useEffect, useRef, useState } from 'react';
import { useViewLocation } from './useViewLocation';
import {
  commandExecution,
  executionFailure,
  readExecution,
  readExecutionTasks,
} from './executionApi';
import { startCommand } from './executionModel';
import type {
  ExecutionCommand,
  LandingResult,
  RunSummary,
} from './executionModel';
import { isFresh, useExecutionResource } from './useExecutionResource';
import { ExecutionReview } from './ExecutionReview';
import { ExecutionLanding } from './ExecutionLanding';
import { ExecutionReplay } from './ExecutionReplay';
import { CandidateDetails } from './ExecutionCandidates';
import type { BoardItem } from './boardModel';
import './styles/board.css';
import './styles/execution.css';

const runLabels: Record<RunSummary['state'], string> = {
  running: '执行中',
  candidate_review: '候选待审',
  candidate_retained: '候选留存，未验收',
  stopped: '已停止',
  failed: '执行失败',
};
const phaseLabels: Record<RunSummary['progress']['phase'], string> = {
  working: '工作中',
  pacing: '节奏检查',
  wrapup: '准备收尾',
  exhausted: '预算耗尽',
};

export function ExecutionBoundary({
  projects,
}: {
  projects: { root: string; name: string }[];
}) {
  const { location, select } = useViewLocation();
  const selected = location.project ?? '';
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [failed, setFailed] = useState(false);
  const [landing, setLanding] = useState<LandingResult | null>(null);
  const pending = useRef<AbortController | null>(null);
  const mounted = useRef(true);
  const state = useExecutionResource(readExecution, revision);
  const project = projects.some((p) => p.root === selected)
    ? selected
    : (projects[0]?.root ?? '');
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      pending.current?.abort();
    };
  }, []);
  const submit = async (command: ExecutionCommand): Promise<boolean> => {
    if (pending.current || !isFresh(state) || !state.data?.configured)
      return false;
    const controller = new AbortController();
    pending.current = controller;
    setBusy(true);
    setMessage('');
    setFailed(false);
    if (command.operation === 'land') setLanding(null);
    const timer = window.setTimeout(() => controller.abort(), 20_000);
    let success = false;
    try {
      const result = await commandExecution(command, controller.signal);
      if (controller.signal.aborted)
        throw new Error(
          '操作响应已过期，结果不可确认。请重新核对后再决定是否提交。',
        );
      if (
        result &&
        state.data.runs.find((run) => run.task_id === result.task_id)
          ?.attempt_id !== result.attempt_id
      )
        throw new Error(
          '落地响应的 attempt 已变化，结果不可确认。请重新核对候选。',
        );
      success = !result || result.applied;
      if (mounted.current) {
        if (result) setLanding(result);
        setFailed(!success);
        setMessage(
          result
            ? result.applied
              ? '服务器已确认代码落地，仍需逐项审阅回执。正在重新核对状态。'
              : '代码未完成落地，请检查落地回执中的冲突与候选。不会自动重试或提交审阅。'
            : '服务器已接受操作，正在重新核对状态。',
        );
      }
    } catch (error) {
      if (mounted.current) {
        setFailed(true);
        setMessage(
          controller.signal.aborted
            ? '未取得操作结果，状态不可确认。正在重新核对；不会自动重试。'
            : executionFailure(error, command),
        );
      }
    } finally {
      window.clearTimeout(timer);
      if (pending.current === controller) pending.current = null;
      if (mounted.current) {
        setBusy(false);
        setRevision((n) => n + 1);
      }
    }
    return success;
  };
  const available = isFresh(state) && state.data?.configured === true;
  return (
    <section className="board-panel execution-panel" aria-label="执行与回放">
      <div className="board-toolbar">
        <label>
          执行项目{' '}
          <select
            value={project}
            disabled={busy}
            onChange={(event) =>
              select({ project: event.target.value, item: null })
            }
          >
            {projects.map((p) => (
              <option key={p.root} value={p.root}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <button
          className="outline-button"
          disabled={busy}
          onClick={() => setRevision((n) => n + 1)}
        >
          核对执行状态
        </button>
      </div>
      <div className="execution-control">
        {state.error ? (
          <p role="alert">{state.error}</p>
        ) : state.loading ? (
          <p role="status">正在核对执行服务…</p>
        ) : state.data ? (
          <>
            <p>
              {!state.data.configured
                ? '执行服务未配置 · 默认冻结'
                : state.data.frozen
                  ? '执行服务已冻结'
                  : '执行服务已解冻'}
            </p>
            {state.data.configured ? (
              <button
                className="outline-button"
                disabled={!available || busy}
                onClick={() => {
                  void submit({
                    operation: 'freeze',
                    frozen: !state.data!.frozen,
                  });
                }}
              >
                {state.data.frozen ? '二次认证后解冻执行' : '冻结执行'}
              </button>
            ) : null}
            <p>
              解冻、代码落地与提交审阅需要在「账户」完成当前有效的二次认证。状态每
              15 秒核对。
            </p>
          </>
        ) : (
          <p role="alert">执行服务状态不可确认。</p>
        )}
        {message ? <p role={failed ? 'alert' : 'status'}>{message}</p> : null}
      </div>
      {state.data?.runs.length ? (
        <section aria-label="执行体进度">
          <h3>执行体进度</h3>
          <ul className="execution-runs">
            {state.data.runs.map((run) => (
              <li key={`${run.task_id}-${run.attempt_id}`}>
                <p>
                  <code>{run.task_id}</code> · {run.adapter} ·{' '}
                  {runLabels[run.state]}
                </p>
                <p>
                  Attempt <code>{run.attempt_id}</code>
                </p>
                <p>
                  {phaseLabels[run.progress.phase]} · 工具调用{' '}
                  {run.progress.tool_calls} 次 · 编辑 {run.progress.edits} 次
                </p>
              </li>
            ))}
          </ul>
        </section>
      ) : state.data?.configured ? (
        <p>当前没有执行体记录。</p>
      ) : null}
      {project ? (
        <ExecutionTasks
          key={project}
          project={project}
          revision={revision}
          busy={busy}
          canStart={available && !state.data?.frozen}
          canCommand={available}
          runs={state.data?.runs ?? []}
          landing={landing}
          submit={submit}
        />
      ) : (
        <p>当前没有可查看的项目。</p>
      )}
    </section>
  );
}
function ExecutionTasks({
  project,
  revision,
  busy,
  canStart,
  canCommand,
  runs,
  landing,
  submit,
}: {
  project: string;
  revision: number;
  busy: boolean;
  canStart: boolean;
  canCommand: boolean;
  runs: RunSummary[];
  landing: LandingResult | null;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const { location, select } = useViewLocation();
  const selected = location.item ?? '';
  const load = useCallback(
    (signal: AbortSignal) => readExecutionTasks(project, signal),
    [project],
  );
  const tasks = useExecutionResource(load, revision);
  const task =
    tasks.data?.find((task) => task.id === selected) ?? tasks.data?.[0];
  const current = isFresh(tasks);
  const command = (request: ExecutionCommand) =>
    current && isFresh(tasks) ? submit(request) : Promise.resolve(false);
  return (
    <section className="execution-tasks" aria-label="任务执行控制">
      <h3>任务执行</h3>
      {tasks.error ? (
        <p role="alert">{tasks.error}</p>
      ) : tasks.loading ? (
        <p role="status">正在读取任务登记…</p>
      ) : !task ? (
        <p>此项目没有已登记任务。</p>
      ) : (
        <>
          <label>
            执行任务{' '}
            <select
              value={task.id}
              disabled={busy}
              onChange={(event) => select({ item: event.target.value })}
            >
              {tasks.data?.map((row) => (
                <option key={row.id} value={row.id}>
                  {row.title} · {row.status}
                </option>
              ))}
            </select>
          </label>
          <TaskControls
            key={task.id}
            task={task}
            run={runs.find((run) => run.task_id === task.id)}
            disabled={busy || !current || !canCommand}
            canStart={canStart}
            submit={command}
          />
          <ExecutionLanding
            key={`landing-${task.id}-${task.attempt?.id ?? 'none'}-${task.candidateReceipt?.manifestRevision ?? 'none'}`}
            task={task}
            run={runs.find((run) => run.task_id === task.id)}
            disabled={busy || !current || !canCommand}
            result={landing}
            submit={command}
          />
          <ExecutionReview
            key={`${task.id}-${task.attempt?.id ?? 'none'}-${task.candidateReceipt?.manifestRevision ?? 'none'}`}
            task={task}
            disabled={
              busy ||
              !current ||
              !canCommand ||
              runs.some(
                (run) =>
                  run.task_id === task.id && run.state === 'candidate_retained',
              )
            }
            submit={command}
          />
          <ExecutionReplay
            key={task.id}
            taskId={task.id}
            project={project}
            revision={revision}
          />
        </>
      )}
    </section>
  );
}
function TaskControls({
  task,
  run,
  disabled,
  canStart,
  submit,
}: {
  task: BoardItem;
  run: RunSummary | undefined;
  disabled: boolean;
  canStart: boolean;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const supported =
    (task.adapter === 'claude' || task.adapter === 'codex') && !!task.creator;
  return (
    <article className="execution-task" aria-label={`执行控制：${task.title}`}>
      <h4>{task.title}</h4>
      <p>
        任务状态：{task.status} · 执行适配器：{task.adapter}
      </p>
      <p>
        登记实例：{task.creator ?? '未登记'} ·{' '}
        {task.attempt?.active ? '当前持有实例' : '上次持有实例'}：
        {task.holder ?? '无'}
      </p>
      {task.attempt ? (
        <p>
          Attempt <code>{task.attempt.id}</code> · fence {task.attempt.fence} ·
          租约至{' '}
          <time dateTime={new Date(task.attempt.leaseUntilMs).toISOString()}>
            {new Date(task.attempt.leaseUntilMs).toLocaleString()}
          </time>
        </p>
      ) : null}
      {task.oldPolicy ? (
        <p>按旧版规则执行；启动仍需服务器核对冻结的规则快照。</p>
      ) : null}
      <p>写入范围：{task.paths.join('、') || '无'}</p>
      <div className="execution-actions">
        <button
          className="outline-button"
          disabled={
            disabled ||
            !canStart ||
            !supported ||
            !['draft', 'queued'].includes(task.status)
          }
          onClick={() => {
            void submit(startCommand(task));
          }}
        >
          启动登记执行体
        </button>
        <button
          className="outline-button"
          disabled={disabled || run?.state !== 'running'}
          onClick={() => {
            void submit({ operation: 'cancel', task_id: task.id });
          }}
        >
          取消执行
        </button>
      </div>
      {!supported ? (
        <p>该任务没有可启动的 Claude / Codex 适配器与登记实例。</p>
      ) : null}
      <CandidateDetails item={task} />
    </article>
  );
}
