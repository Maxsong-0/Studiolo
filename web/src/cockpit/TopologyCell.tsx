import { useId, type ReactNode } from 'react';
import { TopologyDiagram } from '../collab/TopologyDiagram';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  Skeleton,
} from '../ui';
import { overviewNow, type OverviewState } from './model';
import { MINI_TASKS, type PendingPlan } from './pending';
import '../styles/cockpit.css';

interface TopologyCellProps {
  state: OverviewState;
  plan: PendingPlan;
  /** 悬停或聚焦在待我动作某条黑板任务上时，缩略图同步选中它。 */
  selectedTaskId: string | null;
  onSelectTask: (taskId: string) => void;
  onOpenCollab: () => void;
  onRetry: () => void;
}

function TopologyBody({
  state,
  plan,
  selectedTaskId,
  onSelectTask,
  onRetry,
}: TopologyCellProps): ReactNode {
  switch (state.kind) {
    case 'loading':
      return <Skeleton lines={4} label="正在读取黑板汇总…" />;
    case 'error':
      return (
        <ErrorState
          title="黑板汇总读取失败"
          onRetry={onRetry}
          retryLabel="重试黑板汇总"
        >
          <p>{state.message}</p>
        </ErrorState>
      );
    case 'disabled':
      return (
        <EmptyState title="黑板未启用">
          <p>
            此工作台尚未启用协作黑板（汇总接口返回
            404），拓扑与黑板读数暂不可用；其余格子照常。
          </p>
        </EmptyState>
      );
    case 'ready':
      if (plan.miniItems.length === 0)
        return <EmptyState title="黑板上没有未结束的任务" />;
      return (
        <div className="cockpit-topology__canvas">
          <TopologyDiagram
            items={plan.miniItems}
            mode="mini"
            maxTasks={MINI_TASKS}
            showDormant
            onlyConflicts={false}
            markers={plan.markers}
            selectedTaskId={selectedTaskId}
            onSelectTask={onSelectTask}
            title={`协作拓扑缩略：画出 ${plan.miniItems.length} 条未结束任务${
              plan.omitted > 0 ? `，另 ${plan.omitted} 条在协作页` : ''
            }；红圈 ${plan.markers.length} 处等你`}
            now={overviewNow(state.overview)}
          />
        </div>
      );
  }
}

/**
 * 右上：协作拓扑缩略（网格纸底）。执行者 → 任务 → 占用路径，只画未结束的任务，最多 6 条；
 * 等你的任务旁画同号红圈，与左下待我动作的编号对应。黑板未启用（404）时只有这一格显示空态。
 */
export function TopologyCell(props: TopologyCellProps) {
  const { state, plan, onOpenCollab } = props;
  const titleId = useId();
  const drawn = state.kind === 'ready' && plan.miniItems.length > 0;
  return (
    <Cell
      as="section"
      span={9}
      surface="grid"
      className="cockpit-topology"
      aria-labelledby={titleId}
    >
      <CellHeader
        title={<span id={titleId}>协作拓扑，今天</span>}
        meta={
          drawn ? (
            <span className="cockpit-legend">
              <span>实线 认领</span>
              <span className="cockpit-signal">红圈 等你，编号对应左下</span>
            </span>
          ) : null
        }
        action={
          plan.omitted > 0 ? (
            <Button
              variant="text"
              size="sm"
              className="cockpit-link"
              onClick={onOpenCollab}
            >
              另 {plan.omitted} 条，去协作页
            </Button>
          ) : null
        }
      />
      <TopologyBody {...props} />
    </Cell>
  );
}
