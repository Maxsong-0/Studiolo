import { useEffect, useId, useState } from 'react';
import type { ReactNode } from 'react';
import type { Receipt } from '../boardModel';
import type { BoardOverviewItem } from '../boardOverviewModel';
import { Button, EmptyState, StatusMark } from '../ui';
import { cx } from '../ui/dom';
import {
  BOARD_REQUEST_MS,
  boardFailure,
  fetchBoardItems,
} from './boardDetailApi';
import { assessmentLabels, isFinished, receiptLabels } from './boardLabels';
import { agentName, shortId } from './executorGrouping';
import {
  findOverlaps,
  overlapRelation,
  projectName,
  taskStatus,
} from './topologyLayout';
import '../styles/collab.css';

export interface TaskDetailProps {
  /** 选中的条目；null 显示「未选中」。 */
  item: BoardOverviewItem | null;
  /** 同一份汇总里的条目：找写域重叠的对方。 */
  items: readonly BoardOverviewItem[];
  /** 判休眠与租约到期的「此刻」。 */
  now: number;
  /** 执行者标签（与拓扑同一套）；null 表示没有持有者。 */
  executorLabel: string | null;
  onViewInBoard: (item: BoardOverviewItem) => void;
  /** 有可打开的任务书时才给；不给就不显示「打开任务书」。 */
  onOpenTaskBook?: (item: BoardOverviewItem) => void;
}

type ReceiptRead =
  { kind: 'ready'; receipt: Receipt } | { kind: 'unavailable'; reason: string };

/**
 * 拓扑右侧的任务详情：认领、状态、写域、重叠与回执五字段。
 * 汇总接口只给回执结论；五字段要读该项目的黑板明细——只在选中的条目有回执时读一次，
 * 走与「任务与回执」同一个取数与解码；读不到时每项显示「—」并写出原因。
 */
export function TaskDetail({
  item,
  items,
  now,
  executorLabel,
  onViewInBoard,
  onOpenTaskBook,
}: TaskDetailProps) {
  const receiptHeading = useId();
  const read = useReceipt(item);
  if (!item)
    return (
      <div className="task-detail task-detail--empty">
        <EmptyState title="未选中任务">
          <p>点拓扑里的任务框，看它的认领、写域、重叠与回执。</p>
        </EmptyState>
      </div>
    );

  const { status, state } = taskStatus(item, now);
  const others = new Map(items.map((entry) => [entry.id, entry]));
  const overlaps = findOverlaps(items)
    .filter(
      (overlap) => overlap.a.taskId === item.id || overlap.b.taskId === item.id,
    )
    .map((overlap) =>
      overlap.a.taskId === item.id
        ? { own: overlap.a.path, other: overlap.b }
        : { own: overlap.b.path, other: overlap.a },
    );

  const pending = item.receiptStatus !== null && read === null;
  const reason =
    item.receiptStatus === null
      ? isFinished(item.status)
        ? '这条任务结束时没有提交回执。'
        : '任务尚未结束，还没有回执。'
      : read?.kind === 'unavailable'
        ? read.reason
        : null;
  const receipt = read?.kind === 'ready' ? read.receipt : null;
  const fields: { label: string; value: ReactNode; signal?: boolean }[] = [
    {
      label: '状态',
      value: receipt
        ? receiptLabels[receipt.status]
        : item.receiptStatus
          ? receiptLabels[item.receiptStatus]
          : null,
      signal: (receipt?.status ?? item.receiptStatus) === 'failed',
    },
    {
      label: '改动文件',
      value: receipt ? `${receipt.artifacts.length} 个` : null,
    },
    {
      label: '冲突',
      value: receipt ? `${receipt.conflicts.length} 处` : null,
      signal: Boolean(receipt?.conflicts.length),
    },
    {
      label: '验收自评',
      value: receipt ? assessment(receipt) : null,
      signal: Boolean(
        receipt?.acceptance.some((entry) => entry.result === 'fail'),
      ),
    },
    {
      label: '剩余事项',
      value: receipt
        ? receipt.remaining.length
          ? `${receipt.remaining.length} 项：${receipt.remaining[0]}`
          : '无'
        : null,
    },
  ];

  return (
    <div className="task-detail">
      <div className="task-detail__head">
        <span className="task-detail__caption" title={item.id}>
          {shortId(item.id)} · 已选中 · {projectName(item.project)}
        </span>
        <h2 className="task-detail__title">{item.title}</h2>
      </div>
      <dl className="task-detail__facts">
        <div>
          <dt>认领</dt>
          <dd>
            {item.holder === null ? (
              `未认领（${agentName(item.adapter)}）`
            ) : (
              <span>
                {executorLabel ?? agentName(item.adapter)}
                {' / '}
                <code title={item.holder}>{shortId(item.holder)}</code>
              </span>
            )}
          </dd>
        </div>
        <div>
          <dt>状态</dt>
          <dd>
            <StatusMark state={state}>{status}</StatusMark>
            {item.progress ? (
              <span className="task-detail__progress">
                工具 {item.progress.tools} 次 · 编辑 {item.progress.edits} 次 ·
                最近进度{' '}
                <time
                  dateTime={new Date(
                    item.progress.lastProgressMs,
                  ).toISOString()}
                >
                  {new Date(item.progress.lastProgressMs).toLocaleString()}
                </time>
              </span>
            ) : (
              <span className="task-detail__progress">尚无进度回报</span>
            )}
          </dd>
        </div>
        <div>
          <dt>写域</dt>
          <dd className="task-detail__paths">
            {item.paths.length
              ? item.paths.map((path) => <code key={path}>{path}</code>)
              : '只读（无独占路径）'}
          </dd>
        </div>
        <div className={cx(overlaps.length > 0 && 'task-detail--signal')}>
          <dt>重叠</dt>
          <dd>
            {overlaps.length ? (
              <ul className="task-detail__overlaps">
                {overlaps.map(({ own, other }) => {
                  const owner = others.get(other.taskId);
                  const relation = overlapRelation(own, other.path);
                  return (
                    <li key={`${own}\u0000${other.taskId}\u0000${other.path}`}>
                      <code>{other.path}</code>
                      <span>
                        {owner ? owner.title : other.taskId}
                        {owner ? ` · ${taskStatus(owner, now).status}` : ''}；
                        {relation === 'equal'
                          ? `与本任务的 ${own} 相同`
                          : relation === 'contains'
                            ? `落在本任务的 ${own} 里`
                            : `包含本任务的 ${own}`}
                      </span>
                    </li>
                  );
                })}
              </ul>
            ) : (
              '无'
            )}
          </dd>
        </div>
      </dl>
      <section
        className="task-detail__receipt"
        aria-labelledby={receiptHeading}
      >
        <h3 id={receiptHeading}>回执</h3>
        <dl className="task-detail__fields">
          {fields.map((field) => (
            <div
              key={field.label}
              className={cx(field.signal && 'task-detail--signal')}
            >
              <dt>{field.label}</dt>
              <dd>{field.value ?? '—'}</dd>
            </div>
          ))}
        </dl>
        {pending ? (
          <p className="task-detail__reason" role="status">
            正在读取回执明细…
          </p>
        ) : null}
        {reason ? <p className="task-detail__reason">{reason}</p> : null}
      </section>
      <div className="task-detail__actions">
        {onOpenTaskBook ? (
          <Button variant="primary" onClick={() => onOpenTaskBook(item)}>
            打开任务书
          </Button>
        ) : null}
        <Button onClick={() => onViewInBoard(item)}>在任务与回执中查看</Button>
      </div>
    </div>
  );
}

function assessment(receipt: Receipt): string {
  if (!receipt.acceptance.length) return '无';
  const counts = new Map<string, number>();
  for (const entry of receipt.acceptance)
    counts.set(entry.result, (counts.get(entry.result) ?? 0) + 1);
  return (['pass', 'fail', 'unverified'] as const)
    .filter((result) => counts.has(result))
    .map((result) => `${counts.get(result)} ${assessmentLabels[result]}`)
    .join(' · ');
}

/**
 * 选中条目有回执时读一次它所在项目的黑板明细，从中取这条的回执。
 * 换条目或卸载即取消；超时、失败、明细里没有这条，都给出原因而不是空回执。
 */
function useReceipt(item: BoardOverviewItem | null): ReceiptRead | null {
  const project = item?.project ?? null;
  const taskId = item?.id ?? null;
  const wanted = item !== null && item.receiptStatus !== null;
  const key = wanted
    ? `${project}\u0000${taskId}\u0000${item.receiptStatus}`
    : null;
  const [result, setResult] = useState<{
    key: string;
    read: ReceiptRead;
  } | null>(null);
  useEffect(() => {
    if (key === null || project === null || taskId === null) return;
    let disposed = false;
    let settled = false;
    const controller = new AbortController();
    const finish = (read: ReceiptRead) => {
      if (disposed || settled) return;
      settled = true;
      setResult({ key, read });
    };
    const timer = setTimeout(() => {
      finish({
        kind: 'unavailable',
        reason: '回执明细读取超时，结果不可确认。',
      });
      controller.abort();
    }, BOARD_REQUEST_MS);
    void (async () => {
      try {
        const board = await fetchBoardItems(project, controller.signal);
        const match = board.find((entry) => entry.id === taskId);
        finish(
          !match
            ? {
                kind: 'unavailable',
                reason: '黑板明细里没有这条任务，可能已被清理。',
              }
            : match.receipt
              ? { kind: 'ready', receipt: match.receipt }
              : {
                  kind: 'unavailable',
                  reason: '黑板明细里这条任务还没有回执。',
                },
        );
      } catch (error) {
        if (controller.signal.aborted) return;
        finish({ kind: 'unavailable', reason: boardFailure(error) });
      } finally {
        clearTimeout(timer);
      }
    })();
    return () => {
      disposed = true;
      controller.abort();
      clearTimeout(timer);
    };
  }, [key, project, taskId]);
  return key !== null && result?.key === key ? result.read : null;
}
