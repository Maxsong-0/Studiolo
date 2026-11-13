import type { ComponentProps } from 'react';
import { useOverviewRead } from '../../cockpit/useOverviewRead';
import { usePolicyPendingRead } from '../../cockpit/usePolicyPendingRead';
import { CockpitView } from '../../views/CockpitView';

type CockpitProps = ComponentProps<typeof CockpitView>;

/**
 * 不渲染整个 App、单独渲染驾驶舱时，由这里代行 App 的那两份读取（T-162、T-195）：
 * 同样的 useOverviewRead 与 usePolicyPendingRead，快照已在手，刷新规则与 App 相同（一轮刷新结束就重读）。
 * 驾驶舱本身不读黑板汇总和策略草案，只收这两份。
 */
export function CockpitHarness(
  props: Omit<CockpitProps, 'overview' | 'policy'>,
) {
  const reads = { ready: true, refreshing: props.refreshing };
  const overview = useOverviewRead(reads);
  const policy = usePolicyPendingRead(reads);
  return <CockpitView {...props} overview={overview} policy={policy} />;
}
