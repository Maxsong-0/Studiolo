import type { OpenSource, Project, Sourced } from '../types';
import { EmptyState, StatusMark, type StatusState } from '../ui';
import '../styles/projects.css';
import '../styles/projects-mobile.css';

/**
 * 生命周期（后端六值闭集）→ 状态方块，与驾驶舱项目明细同一套：活跃、已交付维护、静态资料是实心，
 * 暂停、候选是空心，未建档是休眠框。闭集外的值后端已拒绝、只会以 null 到达；万一到了，按空心待定处理。
 * 方块只是辅助，文字照样写出来（规范第 1 节）。
 */
const lifecycleStates: Record<string, StatusState> = {
  活跃: 'active',
  已交付维护: 'ok',
  静态资料: 'ok',
  暂停: 'pending',
  候选: 'pending',
  未建档: 'dormant',
};

function lifecycleState(value: string | null): StatusState {
  if (value === null || value === '') return 'dormant';
  return lifecycleStates[value] ?? 'pending';
}

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
  const className = unknown
    ? 'projects-table__value is-unknown'
    : 'projects-table__value';
  return field.source ? (
    <button
      type="button"
      className={className}
      onClick={() => onOpen(field.source!)}
      aria-label={`${label}：${value}，查看来源`}
      title={`${field.source.path}:${field.source.line}`}
    >
      {value}
    </button>
  ) : (
    <span className={className}>{value}</span>
  );
}

/**
 * 项目明细表（规范第 9 节）：序、项目、生命周期、当前版本、进行中、未关闭 BUG、下一里程碑。
 * 点项目名进入运作，点「来源」打开项目原文，点每个数值回到它的原文行；缺失值一律「未知」。
 */
export function ProjectTable({
  projects,
  onOpen,
  onEnter,
}: {
  projects: Project[];
  onOpen: OpenSource;
  /** 点项目名进入该项目的运作；项目来源改由名字旁的「来源」打开。 */
  onEnter: (project: Project) => void;
}) {
  if (projects.length === 0) {
    return (
      <div className="projects-empty" role="status">
        <EmptyState title="没有匹配的项目。可以调整筛选，或检查只读副本。" />
      </div>
    );
  }
  return (
    <div
      className="projects-table-scroll"
      tabIndex={0}
      role="region"
      aria-label="项目列表"
    >
      <table className="projects-table" role="table">
        <colgroup>
          <col className="projects-table__col-index" />
          <col className="projects-table__col-project" />
          <col className="projects-table__col-lifecycle" />
          <col className="projects-table__col-version" />
          <col className="projects-table__col-active" />
          <col className="projects-table__col-bugs" />
          <col />
        </colgroup>
        <thead>
          <tr>
            <th scope="col">序</th>
            <th scope="col">项目</th>
            <th scope="col">生命周期</th>
            <th scope="col">当前版本</th>
            <th scope="col" className="projects-table__number">
              进行中
            </th>
            <th scope="col" className="projects-table__number">
              未关闭 BUG
            </th>
            <th scope="col">下一里程碑</th>
          </tr>
        </thead>
        <tbody>
          {projects.map((project, index) => (
            <tr key={project.id} role="row">
              <td className="projects-table__index" role="cell">
                {String(index + 1).padStart(2, '0')}
              </td>
              <th
                scope="row"
                className="projects-table__identity"
                role="rowheader"
              >
                <button
                  type="button"
                  className="projects-table__name"
                  onClick={() => onEnter(project)}
                  aria-label={`${project.name}，进入项目运作`}
                >
                  {project.name}
                </button>
                {project.profile === 'unfiled' ? (
                  <span className="projects-table__profile">未建档</span>
                ) : null}
                <span className="projects-table__meta">
                  <span className="projects-table__path">{project.root}</span>
                  {project.source ? (
                    <button
                      type="button"
                      className="projects-table__source"
                      onClick={() => onOpen(project.source!)}
                      aria-label={`${project.name}，查看项目来源`}
                      title={`${project.source.path}:${project.source.line}`}
                    >
                      来源
                    </button>
                  ) : null}
                </span>
                {project.diagnostics.length > 0 ? (
                  <span className="sr-only">
                    {project.diagnostics.join('；')}
                  </span>
                ) : null}
              </th>
              <td
                data-label="生命周期"
                className="projects-table__lifecycle"
                role="cell"
              >
                <StatusMark state={lifecycleState(project.lifecycle.value)}>
                  <SourceValue
                    field={project.lifecycle}
                    label={`${project.name} 生命周期`}
                    onOpen={onOpen}
                  />
                </StatusMark>
              </td>
              <td
                data-label="当前版本"
                className="projects-table__version"
                role="cell"
              >
                <SourceValue
                  field={project.version}
                  label={`${project.name} 当前版本`}
                  onOpen={onOpen}
                />
              </td>
              <td
                data-label="进行中"
                className="projects-table__number projects-table__active"
                role="cell"
              >
                <SourceValue
                  field={project.active_tasks}
                  label={`${project.name} 进行中`}
                  onOpen={onOpen}
                />
              </td>
              <td
                data-label="未关闭 BUG"
                className="projects-table__number projects-table__bugs"
                role="cell"
              >
                <SourceValue
                  field={project.open_bugs}
                  label={`${project.name} 未关闭 BUG`}
                  onOpen={onOpen}
                />
              </td>
              <td
                data-label="下一里程碑"
                className="projects-table__milestone"
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
  );
}
