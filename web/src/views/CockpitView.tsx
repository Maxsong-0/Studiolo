import { useId } from 'react';
import type { ReactNode } from 'react';
import { Inbox } from '../components/Inbox';
import { ProjectTable } from '../components/ProjectTable';
import type { Action, OpenSource, Project, Workbench } from '../types';
import { ViewHeader } from './ViewHeader';
import '../styles/workbench.css';
import '../styles/views.css';

/** 驾驶舱只摆项目明细前五行，其余去「项目」看（设计规范第 6 节）。 */
const projectPreview = 5;

/**
 * 驾驶舱（A 轮薄包装）：待我动作全部条目（受筛选）、项目明细前五行与数据提示入口。
 * B 轮 B1 按第 6 节整体重做（巨号、拓扑缩略、时间线、标题栏）。
 */
export function CockpitView({
  filters,
  actions,
  projects,
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
  onRefresh: () => void;
  filters: ReactNode;
  actions: Action[];
  projects: Project[];
  /** 数据提示条数；条目本身在顶栏状态浮层里。 */
  diagnostics: number;
  onOpen: OpenSource;
  onEnterProject: (project: Project) => void;
  onViewProjects: () => void;
  onShowDiagnostics: () => void;
}) {
  const notesTitle = useId();
  const projectsTitle = useId();
  const rest = projects.length - projectPreview;
  return (
    <div className="view view-cockpit">
      <ViewHeader
        title="驾驶舱"
        description="待你验收、拍板与重启的事项和项目概况，回到原文核对。"
      />
      {filters}
      <div className="sheet">
        <div className="sheet-cell sheet-span-8">
          <Inbox actions={actions} onOpen={onOpen} />
        </div>
        <section
          className="sheet-cell sheet-span-4"
          aria-labelledby={notesTitle}
        >
          <div className="cell-heading">
            <h2 id={notesTitle}>数据提示</h2>
          </div>
          {diagnostics ? (
            <>
              <p className="cell-note">
                当前快照有 {diagnostics} 条数据提示：缺失信息保持未知。
              </p>
              <button
                type="button"
                className="text-button"
                onClick={onShowDiagnostics}
              >
                查看数据提示
              </button>
            </>
          ) : (
            <p className="cell-note">当前快照没有数据提示。</p>
          )}
        </section>
        <section
          className="sheet-cell sheet-span-12"
          aria-labelledby={projectsTitle}
        >
          <div className="cell-heading">
            <h2 id={projectsTitle}>项目明细</h2>
            <button
              type="button"
              className="text-button"
              onClick={onViewProjects}
            >
              查看全部项目{rest > 0 ? `（另 ${rest} 项）` : ''}
            </button>
          </div>
          <ProjectTable
            projects={projects.slice(0, projectPreview)}
            onOpen={onOpen}
            onEnter={onEnterProject}
          />
        </section>
      </div>
    </div>
  );
}
