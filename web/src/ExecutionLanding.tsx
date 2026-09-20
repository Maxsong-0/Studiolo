import { useState } from 'react';
import type { BoardItem } from './boardModel';
import { candidatePatch, landingCommand } from './executionModel';
import type {
  ExecutionCommand,
  LandingResult,
  RunSummary,
} from './executionModel';

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
  return (
    <section className="execution-review" aria-label="代码候选落地">
      <h3>代码候选落地</h3>
      <p>
        核对目标仓库当前完整 HEAD 与已封存的 candidate.patch
        SHA-256，再授权服务器应用此候选。提交需要在「账户」完成当前有效的二次认证。
      </p>
      <p>
        候选指针：<code>{patch.path}</code>
        <br />
        SHA-256：<code>{patch.sha256}</code>
      </p>
      <form
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
        <label htmlFor="execution-landing-head">目标仓库完整 HEAD</label>
        <textarea
          id="execution-landing-head"
          rows={2}
          maxLength={64}
          spellCheck={false}
          autoComplete="off"
          value={head}
          disabled={!canLand}
          onChange={(event) => {
            setHead(event.target.value);
            setConfirmed(false);
            setError('');
          }}
        />
        <label htmlFor="execution-landing-sha">candidate.patch SHA-256</label>
        <textarea
          id="execution-landing-sha"
          rows={2}
          maxLength={64}
          spellCheck={false}
          autoComplete="off"
          value={hash}
          disabled={!canLand}
          onChange={(event) => {
            setHash(event.target.value);
            setConfirmed(false);
            setError('');
          }}
        />
        <label className="execution-confirm">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={!canLand}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          我已核对目标 HEAD、候选 SHA-256 与 patch 内容，并授权应用代码。
        </label>
        {error ? <p role="alert">{error}</p> : null}
        {!canLand ? (
          <p>仅当前可确认的候选待审 attempt 可以申请落地，请核对执行状态。</p>
        ) : null}
        <button
          type="submit"
          className="outline-button"
          disabled={!canLand || !confirmed || !head.trim()}
        >
          二次认证后落地代码
        </button>
      </form>
      {receipt ? (
        <section aria-label="本次代码落地回执">
          <h4>本次代码落地回执</h4>
          <p>
            {receipt.applied
              ? receipt.replayed
                ? '已核对此前成功落地，本次未重复应用。'
                : '代码已落地，尚待逐项验收。'
              : '落地未完成；请核对冲突及候选，不能提交为成功。'}
          </p>
          <p>
            核对 HEAD：<code>{receipt.expected_head}</code>
            <br />
            候选目标树：<code>{receipt.after_tree}</code>
          </p>
          {receipt.conflicts.length ? (
            <>
              <p>冲突路径：</p>
              <ul>
                {receipt.conflicts.map((path) => (
                  <li key={path}>
                    <code>{path}</code>
                  </li>
                ))}
              </ul>
            </>
          ) : !receipt.applied ? (
            <p>
              服务器未确认应用完成，且未返回具体冲突路径；请核对回放后处理。
            </p>
          ) : null}
          <p>此回执不代替任务验收，不会自动提交审阅。</p>
        </section>
      ) : null}
    </section>
  );
}
