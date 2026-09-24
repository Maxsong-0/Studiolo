import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { HeroCount } from '../cockpit/HeroCount';
import { PendingActions } from '../cockpit/PendingActions';
import { ProjectDigest } from '../cockpit/ProjectDigest';
import { TimelineCell } from '../cockpit/TimelineCell';
import { TopologyCell } from '../cockpit/TopologyCell';
import { planPending } from '../cockpit/pending';
import { drawingSubtitle, titleCells } from '../cockpit/readings';
import { useCockpitReads } from '../cockpit/useCockpitReads';
import type { OverviewRead } from '../cockpit/useOverviewRead';
import { sortProjects } from '../projectOrder';
import type { Action, OpenSource, Project, Workbench } from '../types';
import { Cell, GridFrame, TitleBlock } from '../ui';
import { useViewLocation } from '../useViewLocation';
import '../styles/cockpit.css';

/** 桌面三行：巨号与拓扑 300px，待我动作与时间线随内容，项目明细与标题栏至少 190px（规范第 6 节）。 */
const ROWS = '300px auto minmax(190px, auto)';

/**
 * 驾驶舱（T-130 设计规范第 6 节，画布「第三轮 融合」主稿）：
 * 左上巨号、右上协作拓扑缩略、左中待我动作（圈号联动拓扑）、右中今天的时间线、
 * 左下项目明细前五行、右下图纸标题栏。
 *
 * 驾驶舱是全局总览：待我动作取整份工作台待办加黑板里等你的任务，项目明细取全部项目按 projectOrder
 * 的前五个，不受项目视图的筛选影响（筛选只在项目视图里，这里没有筛选栏，套用会成为看不见的截断）。
 * 黑板汇总（`/api/v1/board/overview`）由 App 读一次、经 `overview` 交进来，与顶栏「驾驶舱 N」角标共用，
 * 巨号与角标同一口径（T-162）；本机健康由驾驶舱自己读。两路各自成败，任何一处失败只影响依赖它的格子。
 */
export function CockpitView({
  workbench,
  refreshing,
  overview,
  diagnostics,
  onOpen,
  onEnterProject,
  onViewProjects,
  onShowDiagnostics,
}: {
  /** 整份工作台快照（新鲜度、计数、来源、代次）；B 轮驾驶舱的标题栏读数从这里取。 */
  workbench: Workbench;
  /** 快照正在刷新；标题栏据此显示读数是否可能过期。 */
  refreshing: boolean;
  /**
   * 黑板汇总的那一份读取（App 读，顶栏角标用的也是它）：巨号、拓扑、时间线与标题栏都取这里，
   * 拓扑格的「重试」调它的 reload。驾驶舱不再自己另读黑板汇总，刷新后的重读也由 App 那边发。
   */
  overview: OverviewRead;
  onRefresh: () => void;
  /** 项目视图的筛选控件；驾驶舱是全局总览，不摆筛选栏。 */
  filters: ReactNode;
  /** 按项目筛选后的待办；驾驶舱取整份工作台待办，不用这份。 */
  actions: Action[];
  /** 按项目筛选后的项目；驾驶舱取全部项目，不用这份。 */
  projects: Project[];
  /** 数据提示条数；条目本身在顶栏状态浮层里。 */
  diagnostics: number;
  onOpen: OpenSource;
  onEnterProject: (project: Project) => void;
  onViewProjects: () => void;
  onShowDiagnostics: () => void;
}) {
  const { health, reloadHealth } = useCockpitReads();
  const board = overview.state;
  const { goTo } = useViewLocation();
  const [hoveredTask, setHoveredTask] = useState<string | null>(null);
  const projects = useMemo(
    () => sortProjects(workbench.projects),
    [workbench.projects],
  );
  const plan = useMemo(
    () => planPending(board, workbench.inbox, projects),
    [board, workbench.inbox, projects],
  );

  // 一轮刷新结束（成功或失败）就连带重读本机健康：刷新说的是整页读数（黑板汇总由 App 那份读取同规则重读）。
  // 与首次读取同样在提交的同一拍发出，不拖到绘制之后。
  const wasRefreshing = useRef(refreshing);
  useLayoutEffect(() => {
    if (wasRefreshing.current && !refreshing) reloadHealth();
    wasRefreshing.current = refreshing;
  }, [refreshing, reloadHealth]);

  function openCollab(item?: string) {
    goTo(item ? { view: 'collab', item } : { view: 'collab' });
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }

  return (
    <div className="view view-cockpit cockpit">
      <h1 className="sr-only">驾驶舱</h1>
      <GridFrame rows={ROWS} className="cockpit-grid">
        <HeroCount
          total={plan.total}
          oldestWaitMs={plan.oldestWaitMs}
          marked={plan.markers.length}
          truncated={plan.truncated}
          board={board.kind}
        />
        <TopologyCell
          state={board}
          plan={plan}
          selectedTaskId={hoveredTask}
          onSelectTask={(taskId) => openCollab(taskId)}
          onOpenCollab={() => openCollab()}
          onRetry={overview.reload}
        />
        <PendingActions
          entries={plan.entries}
          board={board.kind}
          onOpen={onOpen}
          onOpenTask={(taskId) => openCollab(taskId)}
          onHoverTask={setHoveredTask}
        />
        <TimelineCell state={board} />
        <ProjectDigest
          projects={projects}
          diagnostics={diagnostics}
          onEnterProject={onEnterProject}
          onViewProjects={onViewProjects}
          onShowDiagnostics={onShowDiagnostics}
        />
        <Cell
          as="section"
          span={7}
          flush
          className="cockpit-title"
          aria-label="图纸标题栏"
        >
          <TitleBlock
            title="驾驶舱"
            subtitle={drawingSubtitle(
              Date.now(),
              workbench.generation,
              refreshing,
            )}
            cells={titleCells({
              workbench,
              health,
              overview: board,
            })}
          />
        </Cell>
      </GridFrame>
    </div>
  );
}
