import {
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from 'react';
import { CrewCell } from '../cockpit/CrewCell';
import { QueueCell } from '../cockpit/QueueCell';
import { ReadingsCell } from '../cockpit/ReadingsCell';
import { SinceCell } from '../cockpit/SinceCell';
import { seenTracker, type SeenTracker } from '../cockpit/lastSeen';
import type { LaterStore } from '../cockpit/later';
import { overviewNow } from '../cockpit/model';
import { planPending } from '../cockpit/pending';
import { snapshotLine, systemReadings } from '../cockpit/readings';
import { sinceLastSeen } from '../cockpit/since';
import { useCockpitReads } from '../cockpit/useCockpitReads';
import type { OverviewRead } from '../cockpit/useOverviewRead';
import type { PolicyPendingRead } from '../cockpit/usePolicyPendingRead';
import { overviewLabels } from '../collab/executorGrouping';
import { clockLabel } from '../collab/todayLanes';
import {
  AutoRefreshContext,
  useAutoRefreshReader,
} from '../live/useAutoRefresh';
import type { OpenSource, Workbench } from '../types';
import { useViewLocation } from '../useViewLocation';
import { POLICY_QUEUE_ITEM } from '../viewLocation';
import '../styles/cockpit.css';

/** 图纸上沿的标尺：01 到 12，对应 12 列网格。 */
const RULER = Array.from({ length: 12 }, (_, index) =>
  String(index + 1).padStart(2, '0'),
);

/**
 * 驾驶舱（T-195；设计规范第 6 节，画布「A 进阶」稿）：一张图纸分四格——
 * 左上「等你处理」的队列（巨号与标题同一行），右上「谁在干活」，左下「自上次查看以来」，右下「系统读数」。
 * 左 8 列、右 4 列，12 列网格在够宽时画出来。全页的红只出现在巨号和「停下等你」上。
 *
 * 驾驶舱是全局总览：队列取整份工作台待办、黑板里等你的任务、策略里等你的提案和草案，不受项目视图筛选影响。
 * 黑板汇总（`/api/v1/board/overview`）与策略草案（`/api/v1/policy/desired`）都由 App 各读一次、
 * 经 props 交进来，与顶栏「驾驶舱 N」角标共用，巨号与角标同一口径（T-162）；本机健康由驾驶舱自己读。
 * 三路各自成败，任何一处失败只影响依赖它的格子。
 *
 * 开着时每 20 秒安静重读一轮（T-196，live/autoRefresh.ts）：不闪「读取中」，没读成就留着上一次的数，
 * 「系统读数」标题下那一行写着这些数是多久以前读的。
 *
 * 队列能用键盘走（T-197）：J／K 上下选、Enter 去处理、H 稍后；键位由队列自己登记给全站监听（keys/useGlobalKeys.ts）。
 */
export function CockpitView({
  workbench,
  refreshing,
  overview,
  policy,
  diagnostics,
  onOpen,
  onShowDiagnostics,
  seen = seenTracker(),
  later,
}: {
  /** 整份工作台快照（待办、新鲜度、计数、代次）。 */
  workbench: Workbench;
  /** 快照正在刷新；系统读数据此注明读数可能过期。 */
  refreshing: boolean;
  /** 黑板汇总的那一份读取（App 读，顶栏角标用的也是它）；右栏的「重试」调它的 reload。 */
  overview: OverviewRead;
  /** 策略草案的那一份读取（App 读，顶栏角标用的也是它）；队列里的「重试」调它的 reload。 */
  policy: PolicyPendingRead;
  /** 数据提示条数；条目本身在顶栏状态浮层里。 */
  diagnostics: number;
  onOpen: OpenSource;
  onShowDiagnostics: () => void;
  /** 「上次查看」的记号；缺省用全站那一个，测试里换成自己的。 */
  seen?: SeenTracker;
  /** 队列里「稍后」的记号；缺省用全站那一份，测试里换成自己的。 */
  later?: LaterStore;
}) {
  const { health, reloadHealth, revalidateHealth } = useCockpitReads();
  // 驾驶舱开着时，本机健康跟着自动刷新一起安静重读，排在 App 那三路后面（T-196）。
  useAutoRefreshReader(useContext(AutoRefreshContext), revalidateHealth, 3);
  const board = overview.state;
  const drafts = policy.state;
  const { goTo } = useViewLocation();
  const plan = useMemo(
    () =>
      planPending({
        board,
        policy: drafts,
        inbox: workbench.inbox,
        projects: workbench.projects,
      }),
    [board, drafts, workbench.inbox, workbench.projects],
  );
  // 执行者名按整份汇总算，队列、右栏、协作页叫的是同一个名字。
  const labels = useMemo(
    () => (board.kind === 'ready' ? overviewLabels(board.overview) : undefined),
    [board],
  );
  const committed = workbench.freshness?.committed_through ?? null;

  // 一轮刷新结束（成功或失败）就连带重读本机健康：刷新说的是整页读数（黑板汇总与策略草案由 App 同规则重读）。
  // 与首次读取同样在提交的同一拍发出，不拖到绘制之后。
  const wasRefreshing = useRef(refreshing);
  useLayoutEffect(() => {
    if (wasRefreshing.current && !refreshing) reloadHealth();
    wasRefreshing.current = refreshing;
  }, [refreshing, reloadHealth]);

  // 「上次查看」的基线在这次坐下来期间不变；看到新读数就报给记号，离开页面时由它写下。
  // 黑板还在读或读失败时不报：那一刻你没看到黑板的数。
  const baseline = useSyncExternalStore(seen.subscribe, seen.baseline);
  useEffect(() => {
    if (board.kind === 'ready')
      seen.saw({ at: overviewNow(board.overview), committed });
    else if (board.kind === 'disabled') seen.saw({ at: Date.now(), committed });
  }, [seen, board, committed]);

  const since = sinceLastSeen({
    baseline,
    board,
    policy: drafts,
    freshness: workbench.freshness,
    now: Date.now(),
  });
  // 队列里「等了多久」算到哪一刻：黑板读到了取汇总生成时刻，否则取读到策略草案的那一刻。
  const asOf =
    board.kind === 'ready'
      ? clockLabel(overviewNow(board.overview))
      : drafts.kind === 'ready'
        ? clockLabel(drafts.readAt)
        : null;

  // 这三个去处都交给只随黑板汇总重画的右栏或队列：引用保持不变，免得白白重画。
  const openTask = useCallback(
    (taskId: string) => {
      goTo({ view: 'collab', item: taskId });
      window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
    },
    [goTo],
  );
  const openCollab = useCallback(() => {
    goTo({ view: 'collab' });
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }, [goTo]);
  const openPolicyQueue = useCallback(() => {
    goTo({ view: 'policy', item: POLICY_QUEUE_ITEM });
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }, [goTo]);

  return (
    <div className="view view-cockpit cockpit">
      <h1 className="sr-only">驾驶舱</h1>
      <div className="cockpit-ruler" aria-hidden="true">
        {RULER.map((column) => (
          <span key={column}>{column}</span>
        ))}
      </div>
      <div className="cockpit-sheet">
        <div className="cockpit-sheet__row">
          <QueueCell
            plan={plan}
            board={board}
            policy={drafts}
            labels={labels}
            asOf={asOf}
            onOpen={onOpen}
            onOpenTask={openTask}
            onOpenPolicy={openPolicyQueue}
            onRetryPolicy={policy.reload}
            later={later}
          />
          <CrewCell
            state={board}
            onRetry={overview.reload}
            onOpenCollab={openCollab}
          />
        </div>
        <div className="cockpit-sheet__row">
          <SinceCell since={since} />
          <ReadingsCell
            readings={systemReadings({ workbench, health, overview: board })}
            snapshot={snapshotLine(
              Date.now(),
              workbench.generation,
              refreshing,
            )}
            diagnostics={diagnostics}
            onShowDiagnostics={onShowDiagnostics}
          />
        </div>
      </div>
    </div>
  );
}
