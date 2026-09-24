import type { ComponentProps } from 'react';
import { useOverviewRead } from '../../cockpit/useOverviewRead';
import { CockpitView } from '../../views/CockpitView';

type CockpitProps = ComponentProps<typeof CockpitView>;

/**
 * 不渲染整个 App、单独渲染驾驶舱时，由这里代行 App 的那一份黑板汇总读取（T-162）：
 * 同一个 useOverviewRead，快照已在手，刷新规则与 App 相同（一轮刷新结束就重读）。
 * 驾驶舱本身不读黑板汇总，只收这一份。
 */
export function CockpitHarness(props: Omit<CockpitProps, 'overview'>) {
  const overview = useOverviewRead({
    ready: true,
    refreshing: props.refreshing,
  });
  return <CockpitView {...props} overview={overview} />;
}
