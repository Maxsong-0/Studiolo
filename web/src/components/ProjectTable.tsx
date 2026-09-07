import type { OpenSource, Project, Sourced } from '../types';
import { Icon } from './Icon';
import { PointerSurface } from './PointerSurface';

function SourceValue({
  field,
  label,
  onOpen,
}: {
  field: Sourced<string | number>;
  label: string;
  onOpen: OpenSource;
}) {
  const unknown = field.value === null || field.value === '';
  const value = unknown ? '未知' : field.value;
  return field.source ? (
    <button
      className={`source-value ${unknown ? 'is-unknown' : ''}`}
      onClick={() => onOpen(field.source!)}
      aria-label={`${label}：${value}，查看来源`}
      title={`${field.source.path}:${field.source.line}`}
    >
      {value}
    </button>
  ) : (
    <span className={unknown ? 'is-unknown' : undefined}>{value}</span>
  );
}

export function ProjectTable({
  projects,
  onOpen,
}: {
  projects: Project[];
  onOpen: OpenSource;
}) {
  if (projects.length === 0) {
    return (
      <p className="empty-state" role="status">
        没有匹配的项目。可以调整筛选，或检查只读副本。
      </p>
    );
  }
  return (
    <PointerSurface>
      <div
        className="table-scroll"
        tabIndex={0}
        role="region"
        aria-label="项目列表"
      >
        <table className="project-table" role="table">
          <colgroup>
            <col className="project-column" />
            <col />
            <col />
            <col />
            <col />
            <col className="milestone-column" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">项目</th>
              <th scope="col">生命周期</th>
              <th scope="col">当前版本</th>
              <th scope="col">进行中</th>
              <th scope="col">未关闭 BUG</th>
              <th scope="col">下一里程碑</th>
            </tr>
          </thead>
          <tbody>
            {projects.map((project) => (
              <tr key={project.id} role="row">
                <th scope="row" className="project-identity" role="rowheader">
                  <Icon name="folder" />
                  <div>
                    {project.source ? (
                      <button
                        className="project-name"
                        onClick={() => onOpen(project.source!)}
                        title={`${project.source.path}:${project.source.line}`}
                      >
                        {project.name}
                        <span className="sr-only">，查看项目来源</span>
                      </button>
                    ) : (
                      <span className="project-name-text">{project.name}</span>
                    )}
                    {project.profile === 'unfiled' ? (
                      <span className="project-profile">未建档</span>
                    ) : null}
                    <span className="project-path">{project.root}</span>
                    {project.diagnostics.length > 0 ? (
                      <span className="sr-only">
                        {project.diagnostics.join('；')}
                      </span>
                    ) : null}
                  </div>
                </th>
                <td
                  data-label="生命周期"
                  className={`lifecycle-cell ${project.lifecycle.value === '活跃' ? 'is-active' : ''}`}
                  role="cell"
                >
                  <SourceValue
                    field={project.lifecycle}
                    label={`${project.name} 生命周期`}
                    onOpen={onOpen}
                  />
                </td>
                <td data-label="当前版本" role="cell">
                  <SourceValue
                    field={project.version}
                    label={`${project.name} 当前版本`}
                    onOpen={onOpen}
                  />
                </td>
                <td data-label="进行中" role="cell">
                  <SourceValue
                    field={project.active_tasks}
                    label={`${project.name} 进行中`}
                    onOpen={onOpen}
                  />
                </td>
                <td data-label="未关闭 BUG" role="cell">
                  <SourceValue
                    field={project.open_bugs}
                    label={`${project.name} 未关闭 BUG`}
                    onOpen={onOpen}
                  />
                </td>
                <td
                  data-label="下一里程碑"
                  className="milestone-cell"
                  role="cell"
                >
                  <SourceValue
                    field={project.next_milestone}
                    label={`${project.name} 下一里程碑`}
                    onOpen={onOpen}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </PointerSurface>
  );
}
