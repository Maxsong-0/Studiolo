import { useId } from 'react';
import { BoardBoundary } from '../BoardBoundary';
import { ExecutionBoundary } from '../ExecutionBoundary';
import { ProjectTimeline } from '../ProjectTimeline';
import { useViewLocation } from '../useViewLocation';
import { ViewHeader } from './ViewHeader';
import { ViewTabs } from './ViewTabs';
import '../styles/board.css';
import '../styles/views.css';

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

type ProjectChoice = { root: string; name: string };

/**
 * 协作（A 轮薄包装）：拓扑 · 任务与回执（含本机派发）· 时间线 · 执行。
 * 各标签沿用 BoardBoundary、ProjectTimeline、ExecutionBoundary 的全部行为，项目共用 `project` 位置格；
 * 换标签只换 `panel`，项目与条目留在原处。拓扑在 B 轮接入 `/api/v1/board/overview`。
 */
export function CollabView({ projects }: { projects: ProjectChoice[] }) {
  const { location, select } = useViewLocation();
  const panel = collabPanel(location.panel);
  const id = useId();
  return (
    <div className="view view-collab">
      <ViewHeader
        title="协作"
        description="查看任务归属、路径占用与收尾回执；回放项目时间线，冻结执行并审阅候选产物。"
      />
      <ViewTabs
        label="协作面板"
        tabs={tabs}
        selected={panel}
        idPrefix={id}
        onSelect={(next) =>
          select({ panel: next === 'topology' ? null : next })
        }
      />
      <div
        className="view-tabpanel"
        role="tabpanel"
        id={`${id}-panel`}
        aria-labelledby={`${id}-tab-${panel}`}
      >
        {panel === 'topology' ? (
          <p className="empty-state">协作拓扑将在下一步接入</p>
        ) : null}
        {panel === 'board' ? <BoardBoundary projects={projects} /> : null}
        {panel === 'timeline' ? <CollabTimeline projects={projects} /> : null}
        {panel === 'execution' ? (
          <ExecutionBoundary projects={projects} />
        ) : null}
      </div>
    </div>
  );
}

/** 时间线标签：项目选择与黑板共用 `project` 位置格；换项目整段重挂，旧请求随之取消。 */
function CollabTimeline({ projects }: { projects: ProjectChoice[] }) {
  const { location, select } = useViewLocation();
  const selected = location.project ?? '';
  const project = projects.some((p) => p.root === selected)
    ? selected
    : (projects[0]?.root ?? '');
  return (
    <section className="board-panel" aria-label="时间线">
      <div className="board-toolbar">
        <label>
          项目{' '}
          <select
            value={project}
            onChange={(event) => select({ project: event.target.value })}
          >
            {projects.map((p) => (
              <option key={p.root} value={p.root}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {project ? (
        <ProjectTimeline key={project} project={project} />
      ) : (
        <p>当前没有可查看的项目。</p>
      )}
    </section>
  );
}
