import { useState } from 'react';
import type { BoardItem } from './boardModel';
import type { ExecutionCommand } from './executionModel';
import { Button, Cell, CellHeader } from './ui';
import './styles/execution.css';

/**
 * 丢弃过期的提交：原批准已经过期、一篇正文都没写进 Vault 的提交，只有你能决定丢不丢
 * （2026-11-16 拍板：对账不替你丢，任务一直停在「写回中」）。只在黑板列表实时报了
 * 「提交已过期」的任务上出现；服务器提交时再证实一次，证实不了就不丢。
 */
export function ExecutionDiscard({
  task,
  disabled,
  submit,
}: {
  task: BoardItem;
  disabled: boolean;
  submit: (command: ExecutionCommand) => Promise<boolean>;
}) {
  const [confirmed, setConfirmed] = useState(false);
  if (task.status !== 'committing' || !task.commitExpired) return null;
  return (
    <Cell as="div" className="execution-review-cell">
      <form
        className="execution-review"
        aria-label="丢弃过期的提交"
        onSubmit={(event) => {
          event.preventDefault();
          if (disabled || !confirmed) return;
          void submit({ operation: 'discard_expired', task_id: task.id }).then(
            (ok) => {
              if (ok) setConfirmed(false);
            },
          );
        }}
      >
        <CellHeader title="丢弃过期的提交" level={3} />
        <p className="execution-hint">
          这次提交的原批准已经过期，Broker 核实过一篇正文都还没写进
          Vault。系统不会替你丢：任务一直停在「写回中」，这期间控制库导出和整批备份封存都被挡住。
        </p>
        <p className="execution-hint">
          丢掉之后任务落成失败（停止后交的提交落成取消），暂存的候选正文随即删除、不再保留，不能撤回；要重做就重新启动任务。提交时服务器会再核实一次，核实不了就不丢。需要在「账户」完成当前有效的二次认证。
        </p>
        <label className="execution-check">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={disabled}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          我知道丢掉之后候选正文不再保留，任务落成失败。
        </label>
        <div className="execution-actions">
          <Button
            variant="danger"
            type="submit"
            disabled={disabled || !confirmed}
          >
            丢弃过期的提交
          </Button>
        </div>
      </form>
    </Cell>
  );
}
