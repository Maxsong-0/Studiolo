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
import { statusLabels, statusMark } from './collab/boardLabels';
import { ProjectSelect } from './collab/ProjectSelect';
import {
  EXECUTION_STEPS,
  needsAttention,
  runStep,
  stamp,
  taskStep,
  type ExecutionStep,
} from './executionView';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  Skeleton,
  StatusMark,
  type StatusState,
} from './ui';
import { cx } from './ui/dom';
import './styles/board.css';
import './styles/execution.css';

const runLabels: Record<RunSummary['state'], string> = {
  running: '执行中',
  candidate_review: '候选待审',
  candidate_retained: '候选留存，未验收',
  stopped: '已停止',
  failed: '执行失败',
};
const runMarks: Record<RunSummary['state'], StatusState> = {
  running: 'active',
  candidate_review: 'alert',
  candidate_retained: 'alert',
  stopped: 'dormant',
  failed: 'alert',
};
const phaseLabels: Record<RunSummary['progress']['phase'], string> = {
  working: '工作中',
  pacing: '节奏检查',
  wrapup: '准备收尾',
  exhausted: '预算耗尽',
};

/**
 * 协作 › 执行（设计规范第 7 节，T-130 网格语法）：顶部「执行」与当前任务的 run id，
 * 左侧执行服务的冻结开关，右侧各执行体的九步进度，下面是选中任务的控制、落地、审阅与审计回放。
 * 请求、二次认证与错误文案沿用原样；这里只换版面。
 */
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
  // 任务区当前展示的任务 id：只用来在顶部标出它的 run id。
  const [shown, setShown] = useState<string | null>(null);
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
  const runs = state.data?.runs ?? [];
  const runsCell = runs.length ? (
    <Cell span={8} className="execution-runs-cell" aria-label="执行体进度">
      <CellHeader title="执行体进度" level={3} meta={`${runs.length} 条`} />
      <ul className="execution-runs">
        {runs.map((run) => (
          <li
            key={`${run.task_id}-${run.attempt_id}`}
            className="execution-run"
          >
            <div className="execution-run__who">
              <p className="execution-run__line">
                <code className="execution-run__task">{run.task_id}</code>
                <span className="execution-run__adapter">{run.adapter}</span>
              </p>
              <StatusMark
                state={runMarks[run.state]}
                className="execution-run__state"
              >
                {runLabels[run.state]}
              </StatusMark>
              <p className="execution-run__attempt">
                Attempt <code>{run.attempt_id}</code>
              </p>
            </div>
            <div className="execution-run__progress">
              <StepBar
                progress={runStep(run)}
                label={`执行进度 ${run.task_id}`}
              />
              <p className="execution-run__counts">
                {phaseLabels[run.progress.phase]} · 工具调用{' '}
                {run.progress.tool_calls} 次 · 编辑 {run.progress.edits} 次
              </p>
            </div>
          </li>
        ))}
      </ul>
    </Cell>
  ) : state.data?.configured ? (
    <Cell span={8} as="div" className="execution-runs-cell">
      <CellHeader title="执行体进度" level={3} />
      <EmptyState title="当前没有执行体记录。" />
    </Cell>
  ) : null;
  const shownRun =
    shown === null ? undefined : runs.find((run) => run.task_id === shown);
  return (
    <section className="board-panel execution-panel" aria-label="执行与回放">
      <GridFrame className="execution-grid">
        <Cell as="div" className="execution-head">
          <CellHeader
            title="执行"
            meta={
              <RunId
                run={shownRun}
                reason={
                  shown === null
                    ? '尚未选定任务。'
                    : !state.data
                      ? '执行服务状态不可确认。'
                      : '这条任务在执行服务里没有执行记录。'
                }
              />
            }
          />
          <div className="board-toolbar execution-toolbar">
            <ProjectSelect
              label="执行项目"
              value={project || null}
              options={projects}
              disabled={busy}
              onChange={(root) => {
                if (root) select({ project: root, item: null });
              }}
            />
            <Button
              size="sm"
              disabled={busy}
              onClick={() => setRevision((n) => n + 1)}
            >
              核对执行状态
            </Button>
          </div>
        </Cell>
        <Cell
          as="div"
          span={runsCell ? 4 : undefined}
          className="execution-service"
        >
          <CellHeader title="执行服务" level={3} />
          {state.error ? (
            <ErrorState title={state.error} />
          ) : state.loading ? (
            <Skeleton label="正在核对执行服务…" lines={2} />
          ) : state.data ? (
            <>
              <StatusMark
                className="execution-service__state"
                state={
                  !state.data.configured
                    ? 'dormant'
                    : state.data.frozen
                      ? 'pending'
                      : 'ok'
                }
              >
                {!state.data.configured
                  ? '执行服务未配置 · 默认冻结'
                  : state.data.frozen
                    ? '执行服务已冻结'
                    : '执行服务已解冻'}
              </StatusMark>
              {state.data.configured ? (
                <Button
                  className="execution-service__toggle"
                  disabled={!available || busy}
                  onClick={() => {
                    void submit({
                      operation: 'freeze',
                      frozen: !state.data!.frozen,
                    });
                  }}
                >
                  {state.data.frozen ? '二次认证后解冻执行' : '冻结执行'}
                </Button>
              ) : null}
              <p className="execution-hint">
                解冻、代码落地与提交审阅需要在「账户」完成当前有效的二次认证。状态每
                15 秒核对。
              </p>
            </>
          ) : (
            <ErrorState title="执行服务状态不可确认。" />
          )}
          {message ? (
            <p
              role={failed ? 'alert' : 'status'}
              className={cx(
                'execution-message',
                failed && 'execution-message--failed',
              )}
            >
              {message}
            </p>
          ) : null}
        </Cell>
        {runsCell}
        {project ? (
          <ExecutionTasks
            key={project}
            project={project}
            revision={revision}
            busy={busy}
            canStart={available && !state.data?.frozen}
            canCommand={available}
            runs={runs}
            runsKnown={state.data !== null}
            landing={landing}
            submit={submit}
            onShown={setShown}
          />
        ) : (
          <Cell as="div">
            <EmptyState title="当前没有可查看的项目。" />
          </Cell>
        )}
      </GridFrame>
    </section>
  );
}

/** 顶部标题栏右侧：当前任务在执行服务里的 run（attempt）id；没有就写「—」并说明原因。 */
function RunId({
  run,
  reason,
}: {
  run: RunSummary | undefined;
  reason: string;
}) {
  if (run)
    return (
      <span className="execution-run-id">
        run <code>{run.attempt_id}</code>
      </span>
    );
  return (
    <span className="execution-run-id" title={reason}>
      run —<span className="sr-only">{reason}</span>
    </span>
  );
}

/**
 * 九段进度条：已经过的黑、当前这段红、没到的灰框；回执写入后九段全黑。
 * 读屏读 aria-valuetext（与下面那行字相同）；拿不到进度时不画条，写「—」和原因。
 */
function StepBar({
  progress,
  label,
  named = false,
}: {
  progress: ExecutionStep;
  label: string;
  /** 条下逐段写出段名（选中任务那一条）；窄屏自动收起。 */
  named?: boolean;
}) {
  if (!progress.known)
    return (
      <div className="execution-steps execution-steps--unknown">
        <p className="execution-steps__text" title={progress.reason}>
          九步进度 —
        </p>
        <p className="execution-steps__note">{progress.reason}</p>
      </div>
    );
  const text = `第 ${progress.step} 步，共 ${EXECUTION_STEPS.length} 步。当前：${progress.name}`;
  const segment = (index: number) =>
    progress.finished || index + 1 < progress.step
      ? 'done'
      : index + 1 === progress.step
        ? 'current'
        : 'todo';
  return (
    <div
      className={cx(
        'execution-steps',
        progress.halted && 'execution-steps--halted',
        needsAttention(progress) && 'execution-steps--attention',
      )}
    >
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={EXECUTION_STEPS.length}
        aria-valuenow={progress.step}
        aria-valuetext={text}
        className="execution-steps__bar"
      >
        {EXECUTION_STEPS.map((name, index) => (
          <span
            key={name}
            className="execution-steps__segment"
            data-state={segment(index)}
          />
        ))}
      </div>
      {named ? (
        <ol className="execution-steps__names" aria-hidden="true">
          {EXECUTION_STEPS.map((name, index) => (
            <li key={name} data-state={segment(index)}>
              {name}
            </li>
          ))}
        </ol>
      ) : null}
      <p className="execution-steps__text">{text}</p>
      {progress.note ? (
        <p className="execution-steps__note">{progress.note}</p>
      ) : null}
    </div>
  );
}

function ExecutionTasks({
  project,
  revision,
  busy,
  canStart,
  canCommand,
  runs,
  runsKnown,
  landing,
  submit,
  onShown,
}: {
  project: string;
  revision: number;
  busy: boolean;
  canStart: boolean;
  canCommand: boolean;
  runs: RunSummary[];
  /** 执行服务状态此刻可确认（九步进度据此区分「没有记录」与「不可确认」）。 */
  runsKnown: boolean;
  landing: LandingResult | null;
  submit: (command: ExecutionCommand) => Promise<boolean>;
  onShown: (taskId: string | null) => void;
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
  const shownId = task?.id ?? null;
  useEffect(() => {
    onShown(shownId);
  }, [onShown, shownId]);
  useEffect(() => () => onShown(null), [onShown]);
  return (
    <Cell flush className="execution-tasks" aria-label="任务执行控制">
      {/* 选中任务的一组格子：内层网格不再描外框，分区线由外层网格与内层间隙给出 */}
      <GridFrame className="execution-subgrid">
        <Cell as="div" className="execution-task-cell">
          <CellHeader
            title="任务执行"
            level={3}
            meta={tasks.data ? `${tasks.data.length} 项已登记` : undefined}
          />
          {tasks.error ? (
            <ErrorState title={tasks.error} />
          ) : tasks.loading ? (
            <Skeleton label="正在读取任务登记…" lines={2} />
          ) : !task ? (
            <EmptyState title="此项目没有已登记任务。" />
          ) : (
            <>
              <label className="collab-select execution-task-select">
                执行任务{' '}
                <select
                  className="collab-select__control"
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
                runsKnown={runsKnown}
                landing={landing}
                disabled={busy || !current || !canCommand}
                canStart={canStart}
                submit={command}
              />
            </>
          )}
        </Cell>
        {task ? (
          <>
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
                    run.task_id === task.id &&
                    run.state === 'candidate_retained',
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
        ) : null}
      </GridFrame>
    </Cell>
  );
}
function TaskControls({
  task,
  run,
  runsKnown,
  landing,
  disabled,
  canStart,
  submit,
}: {
  task: BoardItem;
  run: RunSummary | undefined;
  runsKnown: boolean;
  landing: LandingResult | null;
  disabled: boolean;
  canStart: boolean;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const supported =
    (task.adapter === 'claude' || task.adapter === 'codex') && !!task.creator;
  const lease = task.attempt ? stamp(task.attempt.leaseUntilMs) : null;
  return (
    <article className="execution-task" aria-label={`执行控制：${task.title}`}>
      <header className="execution-task__head">
        <h4 className="execution-task__title">{task.title}</h4>
        <StatusMark state={statusMark(task.status)}>
          {statusLabels[task.status]}
        </StatusMark>
      </header>
      <StepBar
        progress={taskStep(task, run, landing, runsKnown)}
        label={`执行进度：${task.title}`}
        named
      />
      <dl className="execution-facts">
        <div>
          <dt>任务状态</dt>
          <dd>
            <code>{task.status}</code>
          </dd>
        </div>
        <div>
          <dt>执行适配器</dt>
          <dd>
            <code>{task.adapter}</code>
          </dd>
        </div>
        <div>
          <dt>登记实例</dt>
          <dd>
            <code>{task.creator ?? '未登记'}</code>
          </dd>
        </div>
        <div>
          <dt>{task.attempt?.active ? '当前持有实例' : '上次持有实例'}</dt>
          <dd>
            <code>{task.holder ?? '无'}</code>
          </dd>
        </div>
        {task.attempt ? (
          <>
            <div>
              <dt>Attempt</dt>
              <dd>
                <code>{task.attempt.id}</code>
              </dd>
            </div>
            <div>
              <dt>fence</dt>
              <dd>
                <code>{task.attempt.fence}</code>
              </dd>
            </div>
            <div>
              <dt>租约至</dt>
              <dd>
                {lease ? <time dateTime={lease.iso}>{lease.text}</time> : '—'}
              </dd>
            </div>
          </>
        ) : null}
        <div>
          <dt>写入范围</dt>
          <dd>
            {task.paths.length ? <code>{task.paths.join('、')}</code> : '无'}
          </dd>
        </div>
      </dl>
      {task.oldPolicy ? (
        <p className="execution-hint">
          按旧版规则执行；启动仍需服务器核对冻结的规则快照。
        </p>
      ) : null}
      <div className="execution-actions">
        <Button
          variant="primary"
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
        </Button>
        <Button
          disabled={disabled || run?.state !== 'running'}
          onClick={() => {
            void submit({ operation: 'cancel', task_id: task.id });
          }}
        >
          取消执行
        </Button>
      </div>
      {!supported ? (
        <p className="execution-hint">
          该任务没有可启动的 Claude / Codex 适配器与登记实例。
        </p>
      ) : null}
      <CandidateDetails item={task} />
    </article>
  );
}
