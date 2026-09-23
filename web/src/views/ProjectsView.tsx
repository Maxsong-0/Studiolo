import type { ReactNode } from 'react';
import { ProjectOperations } from '../components/ProjectOperations';
import { ProjectTable } from '../components/ProjectTable';
import type { OpenSource, Project } from '../types';
import { Button, Cell, GridFrame } from '../ui';
import { useViewLocation } from '../useViewLocation';
import { ViewHeader } from './ViewHeader';
import '../styles/projects.css';

/**
 * 项目（规范第 9 节）：面板 `list`（默认，记作 null）是项目明细表；点项目名进入该项目的
 * `operations`（结构声明、目录漂移、架构、Cargo、Rust 调用），项目沿用 `project` 位置格。
 * 明细表放在 12 列网格的一个整行格子里，筛选行在网格上方（与协作页的子标签行同一层级）。
 */
export function ProjectsView({
  filters,
  projects,
  allProjects,
  generation,
  refreshing,
  onOpen,
  onRefresh,
}: {
  filters: ReactNode;
  /** 按筛选过滤后的项目，给明细表。 */
  projects: Project[];
  /** 全部项目，给运作页的项目选择。 */
  allProjects: Project[];
  generation: number;
  refreshing: boolean;
  onOpen: OpenSource;
  onRefresh: () => void;
}) {
  const { location, select } = useViewLocation();
  if (location.panel === 'operations')
    return (
      <div className="view view-projects">
        <ViewHeader
          title="项目运作"
          description="对照结构声明与实际目录，理解模块职责并回到原文核对。"
        >
          <Button
            variant="text"
            size="sm"
            onClick={() => select({ panel: null, project: null, item: null })}
          >
            <span aria-hidden="true">← </span>返回项目明细
          </Button>
        </ViewHeader>
        <ProjectOperations
          projects={allProjects}
          generation={generation}
          refreshing={refreshing}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      </div>
    );
  return (
    <div className="view view-projects">
      <ViewHeader
        title="项目"
        description="生命周期、版本、进行中与未关闭 BUG；点项目名进入它的运作，点数值回到原文核对。"
      />
      {filters}
      <GridFrame className="projects-sheet">
        <Cell span={12} flush as="div">
          <ProjectTable
            projects={projects}
            onOpen={onOpen}
            onEnter={(project) => {
              select({
                panel: 'operations',
                project: project.root,
                item: null,
              });
              window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
            }}
          />
        </Cell>
      </GridFrame>
    </div>
  );
}
