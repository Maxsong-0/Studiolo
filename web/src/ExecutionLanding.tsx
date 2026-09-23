import { useId, useState } from 'react';
import type { BoardItem } from './boardModel';
import { COMMAND_TARGETS } from './commands/focusTarget';
import { candidatePatch, landingCommand } from './executionModel';
import type {
  ExecutionCommand,
  LandingResult,
  RunSummary,
} from './executionModel';
import { shortHash } from './executionView';
import { Button, Cell, CellHeader, StatusMark } from './ui';
import './styles/execution.css';

/**
 * 代码候选落地（协作 › 执行）。危险按钮前常驻红色虚线框「执行预览」：做什么、不可撤回、要二次认证；
 * 确认仍是原来的「填 HEAD、核对摘要、勾选授权、二次认证后提交」，这里不另加一步。
 * 整格可被 ⌘K 带路聚焦（data-command-target，tabIndex=-1，不进 Tab 序列）。
 */
export function ExecutionLanding({
  task,
  run,
  disabled,
  result,
  submit,
}: {
  task: BoardItem;
  run: RunSummary | undefined;
  disabled: boolean;
  result: LandingResult | null;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const patch = candidatePatch(task);
  const [head, setHead] = useState('');
  const [hash, setHash] = useState(() => patch?.sha256 ?? '');
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState('');
  const previewId = useId();
  const errorId = useId();
  if (!patch || task.status !== 'needs_review' || !task.attempt || task.receipt)
    return null;
  const canLand =
    !disabled &&
    run?.state === 'candidate_review' &&
    run.task_id === task.id &&
    run.attempt_id === task.attempt.id;
  const receipt =
    result?.task_id === task.id &&
    result.attempt_id === task.attempt.id &&
    result.patch_sha256 === patch.sha256
      ? result
      : null;
  const target = head.trim();
  return (
    <Cell
      className="execution-landing"
      aria-label="代码候选落地"
      tabIndex={-1}
      data-command-target={COMMAND_TARGETS.executionLanding}
    >
      <CellHeader
        title="代码候选落地"
        level={3}
        meta={
          <span title={patch.sha256}>
            patch <code>{shortHash(patch.sha256)}</code>
          </span>
        }
      />
      <p className="execution-hint">
        核对目标仓库当前完整 HEAD 与已封存的 candidate.patch
        SHA-256，再授权服务器应用此候选。提交需要在「账户」完成当前有效的二次认证。
      </p>
      <dl className="execution-facts">
        <div>
          <dt>候选指针</dt>
          <dd>
            <code>{patch.path}</code>
          </dd>
        </div>
        <div>
          <dt>SHA-256</dt>
          <dd>
            <code>{patch.sha256}</code>
          </dd>
        </div>
      </dl>
      <form
        className="execution-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (!canLand || !confirmed) return;
          setError('');
          try {
            const command = landingCommand(task, head.trim(), hash.trim());
            setConfirmed(false);
            void submit(command);
          } catch (error) {
            setError(error instanceof Error ? error.message : '落地参数无效。');
          }
        }}
      >
        <div className="execution-field">
          <label htmlFor="execution-landing-head">目标仓库完整 HEAD</label>
          <textarea
            id="execution-landing-head"
            rows={2}
            maxLength={64}
            spellCheck={false}
            autoComplete="off"
            value={head}
            disabled={!canLand}
            aria-describedby={error ? errorId : undefined}
            onChange={(event) => {
              setHead(event.target.value);
              setConfirmed(false);
              setError('');
            }}
          />
        </div>
        <div className="execution-field">
          <label htmlFor="execution-landing-sha">candidate.patch SHA-256</label>
          <textarea
            id="execution-landing-sha"
            rows={2}
            maxLength={64}
            spellCheck={false}
            autoComplete="off"
            value={hash}
            disabled={!canLand}
            aria-describedby={error ? errorId : undefined}
            onChange={(event) => {
              setHash(event.target.value);
              setConfirmed(false);
              setError('');
            }}
          />
        </div>
        {error ? (
          <p role="alert" id={errorId} className="execution-error">
            {error}
          </p>
        ) : null}
        <label className="execution-check">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={!canLand}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          我已核对目标 HEAD、候选 SHA-256 与 patch 内容，并授权应用代码。
        </label>
        {!canLand ? (
          <p className="execution-hint">
            仅当前可确认的候选待审 attempt 可以申请落地，请核对执行状态。
          </p>
        ) : null}
        <div className="execution-preview" id={previewId}>
          <p className="execution-preview__label">执行预览</p>
          <p>
            落地会把已封存的 candidate.patch（SHA-256{' '}
            <code>{shortHash(patch.sha256)}</code>
            …）应用到目标仓库的工作区，前提是仓库当前 HEAD 等于
            {target ? <code>{target}</code> : '上面填写的值'}
            ；只改工作区，不提交、不推送。
          </p>
          <p>
            这是不可撤回的动作，落地后 Studiolo
            不提供撤销。提交需要在「账户」完成当前有效的二次认证（One-time
            Code）；落地成功也不会自动通过任何验收项。
          </p>
        </div>
        <Button
          type="submit"
          variant="danger"
          aria-describedby={previewId}
          disabled={!canLand || !confirmed || !head.trim()}
        >
          二次认证后落地代码
        </Button>
      </form>
      {receipt ? (
        <section
          className="execution-landing-receipt"
          aria-label="本次代码落地回执"
        >
          <h4 className="execution-subhead">本次代码落地回执</h4>
          <StatusMark state={receipt.applied ? 'ok' : 'alert'}>
            {receipt.applied
              ? receipt.replayed
                ? '已核对此前成功落地，本次未重复应用。'
                : '代码已落地，尚待逐项验收。'
              : '落地未完成；请核对冲突及候选，不能提交为成功。'}
          </StatusMark>
          <dl className="execution-facts">
            <div>
              <dt>核对 HEAD</dt>
              <dd>
                <code>{receipt.expected_head}</code>
              </dd>
            </div>
            <div>
              <dt>候选目标树</dt>
              <dd>
                <code>{receipt.after_tree}</code>
              </dd>
            </div>
          </dl>
          {receipt.conflicts.length ? (
            <>
              <p className="execution-subhead">冲突路径：</p>
              <ul className="execution-conflicts">
                {receipt.conflicts.map((path) => (
                  <li key={path}>
                    <code>{path}</code>
                  </li>
                ))}
              </ul>
            </>
          ) : !receipt.applied ? (
            <p className="execution-hint">
              服务器未确认应用完成，且未返回具体冲突路径；请核对回放后处理。
            </p>
          ) : null}
          <p className="execution-hint">
            此回执不代替任务验收，不会自动提交审阅。
          </p>
        </section>
      ) : null}
    </Cell>
  );
}
