import { useId, useMemo, useState } from 'react';
import { BoardBoundary } from '../BoardBoundary';
import { ExecutionBoundary } from '../ExecutionBoundary';
import { ProjectTimeline } from '../ProjectTimeline';
import type { BoardOverview, BoardOverviewItem } from '../boardOverviewModel';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  Lanes,
  Skeleton,
  Tabs,
} from '../ui';
import { cx } from '../ui/dom';
import { useViewLocation } from '../useViewLocation';
import { executorLabels, executorOf } from '../collab/executorGrouping';
import { ProjectSelect } from '../collab/ProjectSelect';
import type { ProjectChoice } from '../collab/ProjectSelect';
import { TaskDetail } from '../collab/TaskDetail';
import { TopologyDiagram } from '../collab/TopologyDiagram';
import { todayLanes } from '../collab/todayLanes';
import {
  projectName,
  topologySummary,
  type TopologySummary,
} from '../collab/topologyLayout';
import {
  OVERVIEW_WINDOW_HOURS,
  useBoardOverview,
  type OverviewState,
} from '../collab/useBoardOverview';
import '../styles/board.css';
import '../styles/collab.css';

const tabs = [
  { id: 'topology', label: '拓扑' },
  { id: 'board', label: '任务与回执' },
  { id: 'timeline', label: '时间线' },
  { id: 'execution', label: '执行' },
] as const;

type CollabPanel = (typeof tabs)[number]['id'];

/** 默认面板拓扑记作 null；未知面板值一律当拓扑，不猜。 */
function collabPanel(value: string | null): CollabPanel {
  return value === 'board' || value === 'timeline' || value === 'execution'
    ? value
    : 'topology';
}

const summaryRule =
  '在跑：未结束且未休眠的任务；写域重叠：未结束任务之间路径相等，或在「/」边界上一条是另一条的前缀；等你：待审阅、等待修订、等待重新确认。';

/**
 * 协作（设计规范第 7 节）：拓扑 · 任务与回执（含本机派发）· 时间线 · 执行。
 * 拓扑读跨项目汇总、缺省看全部项目；其余三个标签沿用 BoardBoundary、ProjectTimeline、
 * ExecutionBoundary 的全部行为，需要先选具体项目。换标签只换 `panel`，项目与条目留在原处。
 */
export function CollabView({ projects }: { projects: ProjectChoice[] }) {
  const { location, select, goTo } = useViewLocation();
  const panel = collabPanel(location.panel);
  const id = useId();
  const [showDormant, setShowDormant] = useState(true);
  const [onlyConflicts, setOnlyConflicts] = useState(false);
  const overview = useBoardOverview(panel === 'topology');
  const topology = useTopology(overview.state, projects, location.project);
  const chosen = projects.some((project) => project.root === location.project)
    ? location.project
    : null;

  const switchPanel = (next: string) => {
    const target = collabPanel(next);
    goTo({
      view: 'collab',
      panel: target === 'topology' ? null : target,
      project: location.project,
      item: location.item,
    });
  };

  return (
    <div className="view view-collab collab">
      <h1 className="sr-only">协作</h1>
      <div className="collab-bar">
        <Tabs
          items={tabs}
          value={panel}
          onChange={switchPanel}
          ariaLabel="协作面板"
          idPrefix={id}
          className="collab-bar__tabs"
        />
        {panel === 'topology' ? (
          <div className="collab-bar__aside">
            <SummaryLine
              summary={topology?.summary ?? null}
              truncated={topology?.overview.itemsTruncated ?? false}
            />
            <label className="collab-toggle">
              <input
                type="checkbox"
                checked={showDormant}
                onChange={(event) => setShowDormant(event.target.checked)}
              />
              显示休眠
            </label>
            <label className="collab-toggle">
              <input
                type="checkbox"
                checked={onlyConflicts}
                onChange={(event) => setOnlyConflicts(event.target.checked)}
              />
              只看冲突
            </label>
          </div>
        ) : null}
      </div>
      <div
        className="collab-panel"
        role="tabpanel"
        id={`${id}-panel-${panel}`}
        aria-labelledby={`${id}-tab-${panel}`}
      >
        {panel === 'topology' ? (
          <TopologyPanel
            state={overview.state}
            reload={overview.reload}
            topology={topology}
            showDormant={showDormant}
            onlyConflicts={onlyConflicts}
            selectedId={location.item}
            onSelect={(taskId) => select({ item: taskId })}
            onFilter={(root) => select({ project: root, item: null })}
            onViewInBoard={(item) =>
              goTo({
                view: 'collab',
                panel: 'board',
                project: item.project,
                item: item.id,
              })
            }
          />
        ) : chosen === null ? (
          <ProjectPrompt
            projects={projects}
            onPick={(root) => select({ project: root, item: null })}
          />
        ) : panel === 'board' ? (
          <BoardBoundary projects={projects} />
        ) : panel === 'timeline' ? (
          <CollabTimeline projects={projects} project={chosen} />
        ) : (
          <ExecutionBoundary projects={projects} />
        )}
      </div>
    </div>
  );
}

interface Topology {
  overview: BoardOverview;
  /** 生成时刻：判休眠与租约到期都按服务端这一刻，不读本机时钟。 */
  now: number;
  /** 按项目筛选后的条目与事件；筛「全部项目」时就是全部。 */
  items: BoardOverviewItem[];
  filter: string | null;
  options: ProjectChoice[];
  summary: TopologySummary;
  labels: Map<string, string>;
  lanes: ReturnType<typeof todayLanes>;
}

/** 拓扑页的派生数据：项目筛选、汇总数、执行者标签与今天的泳道，一份汇总算一次。 */
function useTopology(
  state: OverviewState,
  projects: readonly ProjectChoice[],
  project: string | null,
): Topology | null {
  const overview = state.kind === 'ready' ? state.overview : null;
  return useMemo(() => {
    if (!overview) return null;
    const known = new Map(projects.map((entry) => [entry.root, entry.name]));
    for (const item of overview.items)
      if (!known.has(item.project))
        known.set(item.project, projectName(item.project));
    const options = [...known].map(([root, name]) => ({ root, name }));
    const filter = project !== null && known.has(project) ? project : null;
    const items =
      filter === null
        ? overview.items
        : overview.items.filter((item) => item.project === filter);
    const events =
      filter === null
        ? overview.events
        : overview.events.filter((event) => event.project === filter);
    const now = Date.parse(overview.generatedAt);
    return {
      overview,
      now,
      items,
      filter,
      options,
      summary: topologySummary(items, now),
      labels: executorLabels(items.map(executorOf)),
      lanes: todayLanes(items, events, now),
    };
  }, [overview, projects, project]);
}

/** 右上角的等宽汇总；汇总没读到时每项「—」并说明原因，截断时数字带「+」。 */
function SummaryLine({
  summary,
  truncated,
}: {
  summary: TopologySummary | null;
  truncated: boolean;
}) {
  const shown = (value: number | undefined) =>
    value === undefined ? '—' : `${value}${truncated ? '+' : ''}`;
  const reason =
    summary === null
      ? '黑板汇总还没有读到，数字暂缺。'
      : truncated
        ? '工作项超过 500 条已被截断，数字只统计收到的部分。'
        : null;
  return (
    <p className="collab-summary" title={reason ?? summaryRule}>
      <span>{shown(summary?.running)} 条在跑</span>
      <span className={cx(summary && summary.overlaps > 0 && 'is-signal')}>
        {shown(summary?.overlaps)} 处写域重叠
      </span>
      <span className={cx(summary && summary.waiting > 0 && 'is-signal')}>
        {shown(summary?.waiting)} 处等你
      </span>
      {reason ? <span className="sr-only">{reason}</span> : null}
    </p>
  );
}

function TopologyPanel({
  state,
  reload,
  topology,
  showDormant,
  onlyConflicts,
  selectedId,
  onSelect,
  onFilter,
  onViewInBoard,
}: {
  state: OverviewState;
  reload: () => void;
  topology: Topology | null;
  showDormant: boolean;
  onlyConflicts: boolean;
  selectedId: string | null;
  onSelect: (taskId: string) => void;
  onFilter: (root: string | null) => void;
  onViewInBoard: (item: BoardOverviewItem) => void;
}) {
  if (!topology)
    return (
      <GridFrame className="collab-topology">
        <Cell className="collab-topology__state">
          {state.kind === 'disabled' ? (
            <EmptyState title="黑板未启用">
              <p>
                这台工作台没有装配协作黑板（汇总接口返回
                404），拓扑与今天的时间线无从读取。
              </p>
            </EmptyState>
          ) : state.kind === 'error' ? (
            <ErrorState
              title="实时状态不可确认"
              onRetry={reload}
              retryLabel="重新读取"
            >
              <p>{state.message}</p>
            </ErrorState>
          ) : (
            <Skeleton label="正在读取协作拓扑…" lines={4} />
          )}
        </Cell>
      </GridFrame>
    );

  const { overview, now, items, filter, options, labels, lanes } = topology;
  const selected = selectedId
    ? (items.find((item) => item.id === selectedId) ?? null)
    : null;
  return (
    <GridFrame className="collab-topology">
      <Cell span={9} surface="grid" className="collab-topology__diagram">
        <CellHeader
          title="协作拓扑"
          meta={
            <>
              近 {OVERVIEW_WINDOW_HOURS} 小时 · 截至{' '}
              <time dateTime={overview.generatedAt}>{clock(now)}</time>
            </>
          }
          action={
            <span className="collab-topology__actions">
              <ProjectSelect
                value={filter}
                options={options}
                allLabel="全部项目"
                onChange={onFilter}
              />
              <Button variant="text" size="sm" onClick={reload}>
                重新读取
              </Button>
            </span>
          }
        />
        <TopologyDiagram
          items={items}
          mode="full"
          showDormant={showDormant}
          onlyConflicts={onlyConflicts}
          selectedTaskId={selected?.id ?? null}
          onSelectTask={onSelect}
          title={filter ? `协作拓扑：${projectName(filter)}` : '协作拓扑'}
          now={now}
        />
        <p className="collab-legend">
          <span>实线 认领关系</span>
          <span>虚线框 占用路径</span>
          <span className="is-signal">红色 等你或写域重叠</span>
          <span>灰色虚线 休眠</span>
        </p>
        {overview.itemsTruncated ? (
          <p className="collab-note">
            工作项超过 500 条，接口只给了排在前面的 500
            条；图上缺的条目请到「任务与回执」按项目查看。
          </p>
        ) : null}
      </Cell>
      <Cell
        span={3}
        as="aside"
        className="collab-topology__detail"
        aria-label="任务详情"
      >
        <TaskDetail
          item={selected}
          items={items}
          now={now}
          executorLabel={
            selected ? (labels.get(executorOf(selected).key) ?? null) : null
          }
          onViewInBoard={onViewInBoard}
        />
      </Cell>
      <Cell span={12} className="collab-topology__timeline">
        <CellHeader
          title="今天的时间线"
          meta={
            <span className="collab-lanes-legend">
              <span>黑块 AI 工作</span>
              <span className="is-signal">红块 你的审批</span>
            </span>
          }
        />
        {lanes ? (
          <Lanes
            from={lanes.from}
            to={lanes.to}
            lanes={lanes.lanes}
            ticks={lanes.ticks}
          />
        ) : (
          <EmptyState title="今天还没有黑板事件">
            <p>今天零点以来，所选范围里没有任务事件。</p>
          </EmptyState>
        )}
        {overview.eventsTruncated ? (
          <p className="collab-note">
            窗口内事件超过 1000 条，接口只给了最新的 1000 条，更早的泳道块缺失。
          </p>
        ) : null}
      </Cell>
    </GridFrame>
  );
}

function clock(ms: number): string {
  const date = new Date(ms);
  return `${String(date.getHours()).padStart(2, '0')}:${String(
    date.getMinutes(),
  ).padStart(2, '0')}`;
}

/** 任务与回执、时间线、执行都按项目看：还没选项目时先请人选一个。 */
function ProjectPrompt({
  projects,
  onPick,
}: {
  projects: ProjectChoice[];
  onPick: (root: string) => void;
}) {
  return (
    <GridFrame className="collab-prompt">
      <Cell>
        {projects.length ? (
          <EmptyState
            title="先选一个项目"
            action={
              <ProjectSelect
                value={null}
                options={projects}
                placeholder="选择项目…"
                onChange={(root) => {
                  if (root) onPick(root);
                }}
              />
            }
          >
            <p>
              任务与回执、时间线与执行都按项目查看；拓扑可以一次看全部项目。
            </p>
          </EmptyState>
        ) : (
          <EmptyState title="当前没有可查看的项目。" />
        )}
      </Cell>
    </GridFrame>
  );
}

/** 时间线标签：项目选择与黑板共用 `project` 位置格；换项目整段重挂，旧请求随之取消。 */
function CollabTimeline({
  projects,
  project,
}: {
  projects: ProjectChoice[];
  project: string;
}) {
  const { select } = useViewLocation();
  return (
    <section className="board-panel" aria-label="时间线">
      <ProjectTimeline
        key={project}
        project={project}
        leading={
          <ProjectSelect
            value={project}
            options={projects}
            onChange={(root) => {
              if (root) select({ project: root, item: null });
            }}
          />
        }
      />
    </section>
  );
}
