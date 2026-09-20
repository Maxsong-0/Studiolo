import { useState } from 'react';
import type { BoardItem } from './boardModel';
import {
  candidatePatch,
  reviewReceipt,
  reviewTemplate,
} from './executionModel';
import type { ExecutionCommand } from './executionModel';

export function ExecutionReview({
  task,
  disabled,
  submit,
}: {
  task: BoardItem;
  disabled: boolean;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const [source, setSource] = useState(() => reviewTemplate(task));
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState('');
  if (task.status !== 'needs_review' || !task.candidateReceipt || task.receipt)
    return null;
  return (
    <form
      className="execution-review"
      onSubmit={(event) => {
        event.preventDefault();
        if (disabled || !confirmed) return;
        setError('');
        let receipt: unknown;
        try {
          receipt = reviewReceipt(source, task);
        } catch (error) {
          setError(error instanceof Error ? error.message : '回执无效。');
          return;
        }
        void submit({ operation: 'review', task_id: task.id, receipt }).then(
          (ok) => {
            if (ok) setConfirmed(false);
          },
        );
      }}
    >
      <h3>逐项审阅候选</h3>
      <p>
        先核对候选文件和证据，再填写回执。此处只校验结构；产物内容、验收结论与经验去向需要你确认。提交需要在「账户」完成当前有效的二次认证。
      </p>
      {candidatePatch(task) ? (
        <p>
          代码候选须先完成上方的代码落地，再提交 completed
          回执；服务器会重新核对落地状态。落地成功不会自动通过任何验收项。
        </p>
      ) : null}
      <p>
        acceptance 按任务原顺序逐条填写 pass / fail / unverified 与 evidence
        指针；artifacts 使用候选产物的 path、revision、sha256。status 选择
        completed / failed / stopped / abandoned。填写实际
        elapsed_ms、tool_calls，以及 closeout 的
        learned、adr_required、rule_change_required、destinations。completed
        仅允许全部 pass，且 conflicts、unlanded、remaining 为空。
      </p>
      <label htmlFor="execution-receipt">验收回执 JSON</label>
      <textarea
        id="execution-receipt"
        rows={20}
        spellCheck={false}
        value={source}
        disabled={disabled}
        onChange={(event) => {
          setSource(event.target.value);
          setConfirmed(false);
          setError('');
        }}
      />
      <label className="execution-confirm">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={disabled}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        我已逐条核对验收项、证据、未落地事项与 closeout。
      </label>
      {error ? <p role="alert">{error}</p> : null}
      <button
        className="outline-button"
        type="submit"
        disabled={disabled || !confirmed}
      >
        提交审阅回执
      </button>
    </form>
  );
}
