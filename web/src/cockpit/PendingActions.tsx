import { useId } from 'react';
import type { OpenSource } from '../types';
import { Cell, CellHeader, CircledNumber } from '../ui';
import type { OverviewState } from './model';
import { actionTitle, type PendingEntry } from './pending';
import '../styles/cockpit.css';

/** 黑板汇总没到手时，列表只有工作台待办；在列表上方说清楚少了哪一块。 */
function boardNote(board: OverviewState['kind']): string | null {
  if (board === 'loading') return '正在读取黑板里等你的任务…';
  if (board === 'error') return '黑板汇总读取失败，等你的黑板任务暂未列出。';
  return null;
}

/**
 * 左中：待我动作。一行一件：圈号＋类型＋标题＋项目。
 * 黑板里等你的任务在前，画进拓扑缩略的圈号红色实心，点开去协作页看这条任务；
 * 工作台待办在后，圈号黑色空心，点开原文（可访问名沿用「标题，查看原文」）。
 */
export function PendingActions({
  entries,
  board,
  onOpen,
  onOpenTask,
  onHoverTask,
}: {
  entries: readonly PendingEntry[];
  board: OverviewState['kind'];
  onOpen: OpenSource;
  onOpenTask: (taskId: string) => void;
  /** 悬停或聚焦到画在缩略图上的任务时报告它，离开时报 null。 */
  onHoverTask: (taskId: string | null) => void;
}) {
  const titleId = useId();
  const note = boardNote(board);
  const settled = note === null;
  return (
    <Cell
      as="section"
      span={6}
      className="cockpit-pending"
      aria-labelledby={titleId}
    >
      <CellHeader
        title={<span id={titleId}>待我动作</span>}
        meta={settled ? `共 ${entries.length} 项` : null}
      />
      {note ? (
        <p className="cockpit-note" role="status">
          {note}
        </p>
      ) : null}
      {entries.length === 0 ? (
        settled ? (
          <p className="cockpit-empty" role="status">
            当前没有待我动作。
          </p>
        ) : null
      ) : (
        <ol className="cockpit-pending__list">
          {entries.map((entry) => {
            if (entry.source === 'board') {
              const { item, marked } = entry;
              const hover = marked
                ? {
                    onMouseEnter: () => onHoverTask(item.id),
                    onMouseLeave: () => onHoverTask(null),
                    onFocus: () => onHoverTask(item.id),
                    onBlur: () => onHoverTask(null),
                  }
                : {};
              return (
                <li key={`board-${item.id}`}>
                  <button
                    type="button"
                    className="cockpit-pending__row"
                    data-source="board"
                    aria-label={`${item.title}，在协作页查看`}
                    title={`${item.title}（${item.project}）`}
                    onClick={() => onOpenTask(item.id)}
                    {...hover}
                  >
                    <CircledNumber n={entry.n} filled={marked} />
                    <span
                      className={
                        marked
                          ? 'cockpit-pending__kind cockpit-signal'
                          : 'cockpit-pending__kind'
                      }
                    >
                      {entry.kind}
                    </span>
                    <span className="cockpit-pending__title">{item.title}</span>
                    <span className="cockpit-pending__project">
                      {entry.project}
                    </span>
                  </button>
                </li>
              );
            }
            const { action } = entry;
            return (
              <li key={`inbox-${action.id}`}>
                <button
                  type="button"
                  className="cockpit-pending__row"
                  data-source="inbox"
                  aria-label={`${action.title}，查看原文`}
                  title={`${action.source.path} · 第 ${action.source.line} 行`}
                  onClick={() => onOpen(action.source)}
                >
                  <CircledNumber n={entry.n} />
                  <span className="cockpit-pending__kind">
                    {entry.kind || '未知'}
                  </span>
                  <span className="cockpit-pending__title">
                    {actionTitle(action)}
                  </span>
                  <span className="cockpit-pending__project">
                    {entry.project}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      )}
    </Cell>
  );
}
