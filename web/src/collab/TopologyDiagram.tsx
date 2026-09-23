import { useId, useMemo, useState } from 'react';
import type { JSX } from 'react';
import type { BoardOverviewItem } from '../boardOverviewModel';
import { EmptyState, StatusMark } from '../ui';
import { cx } from '../ui/dom';
import { useViewLocation } from '../useViewLocation';
import { COLUMNS, topologyLayout } from './topologyLayout';
import type {
  MarkerPlacement,
  TaskRow,
  TopologyLayout,
} from './topologyLayout';
import '../styles/collab.css';

/** 圈号：n 是驾驶舱待办的序号，target 指到一条任务或一条占用路径。 */
export interface TopologyMarker {
  n: number;
  target: { taskId: string } | { path: string };
}

export interface TopologyDiagramProps {
  /** 汇总接口的条目；已结束的不画。 */
  items: BoardOverviewItem[];
  /** full＝协作页整张图；mini＝驾驶舱缩略（最多 maxTasks 条，超出给「另 N 条，去协作页」）。 */
  mode: 'mini' | 'full';
  maxTasks?: number;
  /** 画不画休眠任务；缺省 full 画、mini 不画。 */
  showDormant?: boolean;
  onlyConflicts?: boolean;
  markers?: TopologyMarker[];
  selectedTaskId?: string | null;
  onSelectTask?: (taskId: string) => void;
  /** 整张图的名字：进 aria-label，也是清单的说明。 */
  title: string;
  /** 判休眠与租约到期的「此刻」；缺省取挂载时刻。 */
  now?: number;
}

/**
 * 协作拓扑图（设计规范第 7 节）：执行者 → 任务 → 占用路径三列的 SVG，另附一份按执行者分组的清单。
 * SVG 是一幅图（role=img）；清单平时视觉隐藏、供读屏与键盘（聚焦进来时展开），窄屏只显示清单。
 */
export function TopologyDiagram(props: TopologyDiagramProps): JSX.Element {
  const {
    items,
    mode,
    maxTasks,
    showDormant,
    onlyConflicts = false,
    markers,
    selectedTaskId = null,
    onSelectTask,
    title,
    now,
  } = props;
  const [mountedAt] = useState(() => Date.now());
  const at = now ?? mountedAt;
  const dormantShown = showDormant ?? mode === 'full';
  const layout = useMemo(
    () =>
      topologyLayout(items, {
        mode,
        now: at,
        maxTasks,
        showDormant: dormantShown,
        onlyConflicts,
        markers,
        selectedTaskId,
      }),
    [
      items,
      mode,
      at,
      maxTasks,
      dormantShown,
      onlyConflicts,
      markers,
      selectedTaskId,
    ],
  );
  // useId 的值带冒号或书名号，放进 url(#…) 前只留安全字符。
  const arrow = `topology-arrow-${useId().replace(/[^A-Za-z0-9_-]/g, '')}`;
  const { goTo } = useViewLocation();
  const more =
    mode === 'mini' && layout.omitted > 0 ? (
      <p className="topology__more">
        <button
          type="button"
          className="topology__more-link"
          onClick={() => goTo({ view: 'collab' })}
        >
          另 {layout.omitted} 条，去协作页
        </button>
      </p>
    ) : null;

  if (layout.tasks.length === 0)
    return (
      <div className={cx('topology', `topology--${mode}`)}>
        <EmptyState title={emptyTitle(layout, onlyConflicts)}>
          {emptyHint(layout, onlyConflicts, dormantShown)}
        </EmptyState>
        {more}
      </div>
    );

  return (
    <div className={cx('topology', `topology--${mode}`)}>
      <svg
        className="topology__svg"
        role="img"
        aria-label={title}
        viewBox={`0 0 ${layout.width} ${layout.height}`}
        preserveAspectRatio="xMidYMin meet"
      >
        <defs>
          {/* 箭头按用户坐标定大小：选中任务的连线加粗时箭头不跟着放大 */}
          <marker
            id={arrow}
            viewBox="0 0 8 8"
            refX={7}
            refY={4}
            markerWidth={9}
            markerHeight={9}
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path className="topology__arrow" d="M0 0 L8 4 L0 8 Z" />
          </marker>
        </defs>
        <g className="topology__heads">
          <text x={COLUMNS.executor.x} y={10}>
            执行者
          </text>
          <text x={COLUMNS.task.x} y={10}>
            任务
          </text>
          <text x={COLUMNS.path.x} y={10}>
            占用路径
          </text>
        </g>
        <g className="topology__edges">
          {layout.edges.map((edge) => (
            <path
              key={edge.key}
              className={cx(
                'topology__edge',
                edge.emphasis && 'topology__edge--emphasis',
                edge.dormant && 'topology__edge--dormant',
              )}
              d={edge.d}
              markerEnd={
                edge.kind === 'claim' && !edge.dormant
                  ? `url(#${arrow})`
                  : undefined
              }
            />
          ))}
        </g>
        <g className="topology__executors">
          {layout.executors.map((executor) => (
            <g
              key={executor.key}
              className={cx(
                'topology__executor',
                executor.dormant && 'topology__executor--dormant',
              )}
            >
              <title>{`${executor.label}（${executor.detail}）`}</title>
              <rect
                className="topology__executor-box"
                x={executor.x + 0.75}
                y={executor.y}
                width={executor.width - 1.5}
                height={executor.height}
              />
              <text
                className="topology__executor-name"
                x={executor.x + 12}
                y={executor.y + 19}
              >
                {executor.name}
              </text>
              <text
                className="topology__executor-detail"
                x={executor.x + 12}
                y={executor.y + 35}
              >
                {executor.detail}
              </text>
            </g>
          ))}
        </g>
        <g className="topology__tasks">
          {layout.tasks.map((task) => (
            <g
              key={task.id}
              data-task-id={task.id}
              className={cx(
                'topology__task',
                `topology__task--${task.tone}`,
                task.selected && 'topology__task--selected',
                onSelectTask && 'topology__task--clickable',
              )}
              onClick={onSelectTask ? () => onSelectTask(task.id) : undefined}
            >
              <title>{`${task.item.title}\n${task.item.project} · ${task.status}`}</title>
              <rect
                className="topology__task-box"
                x={task.x}
                y={task.y}
                width={task.width}
                height={task.height}
              />
              <text
                className="topology__task-caption"
                x={task.x + 12}
                y={task.y + 14}
              >
                {task.caption}
              </text>
              <text
                className="topology__task-status"
                x={task.x + task.width - 10}
                y={task.y + 14}
                textAnchor="end"
              >
                {task.statusText}
              </text>
              <text
                className="topology__task-title"
                x={task.x + 12}
                y={task.y + 33}
              >
                {task.titleText}
              </text>
            </g>
          ))}
        </g>
        <g className="topology__paths">
          {layout.paths.map((path) => (
            <g
              key={path.key}
              className={cx(
                'topology__path',
                path.dormant && 'topology__path--dormant',
                path.overlaps.length > 0 && 'topology__path--overlap',
              )}
            >
              <title>
                {path.note ? `${path.path}\n${path.note}` : path.path}
              </title>
              <rect
                className="topology__path-box"
                x={path.x}
                y={path.y}
                width={path.width}
                height={path.height}
              />
              <text
                className="topology__path-text"
                x={path.x + 12}
                y={path.y + 19}
              >
                {path.text}
              </text>
              {path.noteText !== null && path.noteY !== null ? (
                <text className="topology__path-note" x={path.x} y={path.noteY}>
                  {path.noteText}
                </text>
              ) : null}
            </g>
          ))}
        </g>
        <g className="topology__brackets">
          {layout.brackets.map((bracket) => (
            <path
              key={bracket.key}
              className="topology__bracket"
              d={bracket.d}
            />
          ))}
        </g>
        <g className="topology__markers">
          {layout.markers.map((marker) => (
            <g key={marker.key} className="topology__marker">
              <title>{marker.label}</title>
              <line
                className="topology__marker-leader"
                x1={marker.leader.x1}
                y1={marker.leader.y1}
                x2={marker.leader.x2}
                y2={marker.leader.y2}
              />
              <circle
                className="topology__marker-dot"
                cx={marker.cx}
                cy={marker.cy}
                r={marker.r}
              />
              <text
                className="topology__marker-n"
                x={marker.cx}
                y={marker.cy + 4}
                textAnchor="middle"
              >
                {marker.n}
              </text>
            </g>
          ))}
        </g>
      </svg>
      <TopologyOutline
        layout={layout}
        title={title}
        onSelectTask={onSelectTask}
      />
      {more}
    </div>
  );
}

function emptyTitle(layout: TopologyLayout, onlyConflicts: boolean): string {
  if (layout.unfinished === 0) return '黑板上没有未结束的任务';
  if (onlyConflicts && layout.overlaps.length === 0) return '没有写域重叠';
  return '当前开关下没有要画的任务';
}

function emptyHint(
  layout: TopologyLayout,
  onlyConflicts: boolean,
  dormantShown: boolean,
): string | null {
  if (layout.unfinished === 0) return null;
  if (onlyConflicts && layout.overlaps.length === 0)
    return '未结束的任务之间，占用路径两两都不相交。';
  if (!dormantShown && layout.dormant.length > 0)
    return `另有 ${layout.dormant.length} 条休眠任务，打开「显示休眠」可以看到。`;
  return null;
}

/** 按执行者分组的文字清单：读屏的主入口、键盘选任务的入口，也是窄屏的唯一呈现。 */
function TopologyOutline({
  layout,
  title,
  onSelectTask,
}: {
  layout: TopologyLayout;
  title: string;
  onSelectTask?: (taskId: string) => void;
}) {
  const taskMarkers = (task: TaskRow) =>
    layout.markers.filter(
      (marker) => 'taskId' in marker.target && marker.target.taskId === task.id,
    );
  const pathMarkers = (key: string) =>
    layout.markers.filter(
      (marker) => 'pathKey' in marker.target && marker.target.pathKey === key,
    );
  return (
    <div className="topology__outline">
      <ul className="topology__groups" aria-label={`${title}：按执行者列出`}>
        {layout.executors.map((executor) => (
          <li key={executor.key} className="topology__group">
            <p
              className={cx(
                'topology__group-name',
                executor.dormant && 'topology__group-name--dormant',
              )}
            >
              {executor.label}
              <span className="topology__group-detail">{executor.detail}</span>
            </p>
            <ul
              className="topology__entries"
              aria-label={`${executor.label} 的任务`}
            >
              {layout.tasks
                .filter((task) => task.executorKey === executor.key)
                .map((task) => {
                  const own = layout.paths.filter(
                    (path) => path.taskId === task.id,
                  );
                  return (
                    <li
                      key={task.id}
                      className={cx(
                        'topology__entry',
                        `topology__entry--${task.tone}`,
                      )}
                    >
                      {onSelectTask ? (
                        <button
                          type="button"
                          className="topology__entry-select"
                          aria-pressed={task.selected}
                          onClick={() => onSelectTask(task.id)}
                        >
                          {task.item.title}
                        </button>
                      ) : (
                        <span className="topology__entry-title">
                          {task.item.title}
                        </span>
                      )}
                      <StatusMark
                        className="topology__entry-status"
                        state={task.state}
                      >
                        {task.status}
                      </StatusMark>
                      <MarkerNotes markers={taskMarkers(task)} />
                      <ul
                        className="topology__entry-paths"
                        aria-label="占用路径"
                      >
                        {own.length ? (
                          own.map((path) => (
                            <li key={path.key}>
                              <code>{path.path}</code>
                              <MarkerNotes markers={pathMarkers(path.key)} />
                              {path.note ? (
                                <span className="topology__entry-overlap">
                                  {path.note}
                                </span>
                              ) : null}
                            </li>
                          ))
                        ) : (
                          <li className="topology__entry-none">
                            {task.item.paths.length
                              ? '其余路径与写域重叠无关'
                              : '无独占路径'}
                          </li>
                        )}
                      </ul>
                    </li>
                  );
                })}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  );
}

function MarkerNotes({ markers }: { markers: MarkerPlacement[] }) {
  if (!markers.length) return null;
  return (
    <span className="topology__entry-markers">
      {markers.map((marker) => (
        <span key={marker.key} className="topology__entry-marker">
          待办 {marker.n}
        </span>
      ))}
    </span>
  );
}
