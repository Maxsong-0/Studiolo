import { useOverviewRead } from '../../cockpit/useOverviewRead';
import { collabPanel } from '../../collab/panels';
import type { ProjectChoice } from '../../collab/ProjectSelect';
import { useViewLocation } from '../../useViewLocation';
import { CollabView } from '../../views/CollabView';

/**
 * 单独挂协作页用：黑板汇总那一份读取本来由 App 给（T-196）。
 * 这里照 App 在「还没有工作台快照」时的规矩读：正在看拓扑才读，别的标签不读。
 */
export function CollabHarness({ projects }: { projects: ProjectChoice[] }) {
  const { location } = useViewLocation();
  const overview = useOverviewRead({
    ready: collabPanel(location.panel) === 'topology',
    refreshing: false,
  });
  return <CollabView projects={projects} overview={overview} />;
}
