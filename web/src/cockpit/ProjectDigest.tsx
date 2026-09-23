import { useId } from 'react';
import type { Project } from '../types';
import { Button, Cell, CellHeader, StatusMark, type StatusState } from '../ui';
import '../styles/cockpit.css';

/** 驾驶舱只摆项目明细前五行，其余去「项目」看（设计规范第 6 节第 5 条）。 */
export const PROJECT_PREVIEW = 5;

/**
 * 生命周期 → 状态方块（任务书给定前四档与 null；闭集另两值按同类归档）：
 * 活跃→运行；已交付维护、静态资料→正常；暂停、候选→待定；未建档与未知→休眠。
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
  return value !== null && Object.hasOwn(lifecycleStates, value)
    ? lifecycleStates[value]
    : 'dormant';
}

/** 备注：下一里程碑；没有就给第一条诊断；都没有显示「—」。 */
function projectNote(project: Project): { text: string; title: string } {
  const milestone = project.next_milestone.value;
  if (milestone) return { text: milestone, title: `下一里程碑：${milestone}` };
  if (project.diagnostics.length > 0)
    return {
      text: project.diagnostics[0],
      title: project.diagnostics.join('；'),
    };
  return { text: '—', title: '下一里程碑未知' };
}

/**
 * 左下：项目明细前五行（序号、项目、状态方块、备注），标题右侧「全部 N 个项目」进项目视图，
 * 不静默截断（规范第 5 节）。项目名进入该项目的运作；数据提示只报条数，条目在状态浮层里。
 */
export function ProjectDigest({
  projects,
  diagnostics,
  onEnterProject,
  onViewProjects,
  onShowDiagnostics,
}: {
  /** 已按 projectOrder 排好的全部项目。 */
  projects: readonly Project[];
  /** 数据提示条数。 */
  diagnostics: number;
  onEnterProject: (project: Project) => void;
  onViewProjects: () => void;
  onShowDiagnostics: () => void;
}) {
  const titleId = useId();
  const total = projects.length;
  return (
    <Cell
      as="section"
      span={5}
      className="cockpit-projects"
      aria-labelledby={titleId}
    >
      <CellHeader
        title={<span id={titleId}>项目明细</span>}
        action={
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            aria-label={`全部 ${total} 个项目，打开项目`}
            onClick={onViewProjects}
          >
            全部 {total} 个项目
          </Button>
        }
      />
      {total === 0 ? (
        <p className="cockpit-empty" role="status">
          当前快照没有可读项目。
        </p>
      ) : (
        <table className="cockpit-projects__table">
          <colgroup>
            <col className="cockpit-projects__col-index" />
            <col />
            <col className="cockpit-projects__col-state" />
            <col />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">序号</th>
              <th scope="col">项目</th>
              <th scope="col">状态</th>
              <th scope="col">备注</th>
            </tr>
          </thead>
          <tbody>
            {projects.slice(0, PROJECT_PREVIEW).map((project, index) => {
              const note = projectNote(project);
              return (
                <tr key={project.id}>
                  <td className="cockpit-projects__index">
                    {String(index + 1).padStart(2, '0')}
                  </td>
                  <th scope="row" className="cockpit-projects__name">
                    <button
                      type="button"
                      aria-label={`${project.name}，进入项目运作`}
                      title={project.root}
                      onClick={() => onEnterProject(project)}
                    >
                      {project.name}
                    </button>
                    {project.diagnostics.length > 0 ? (
                      <span className="sr-only">
                        {project.diagnostics.join('；')}
                      </span>
                    ) : null}
                  </th>
                  <td className="cockpit-projects__state">
                    <StatusMark state={lifecycleState(project.lifecycle.value)}>
                      {project.lifecycle.value ?? '未知'}
                    </StatusMark>
                  </td>
                  <td className="cockpit-projects__note" title={note.title}>
                    {note.text}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {diagnostics > 0 ? (
        <p className="cockpit-diagnostics">
          当前快照有 {diagnostics} 条数据提示：缺失信息保持未知。
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            onClick={onShowDiagnostics}
          >
            查看数据提示
          </Button>
        </p>
      ) : null}
    </Cell>
  );
}
