import type { OpenSource, Project } from '../types';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  Skeleton,
} from '../ui';
import { useViewLocation } from '../useViewLocation';
import { useProjectOperations } from '../useProjectOperations';
import { OperationResult } from './OperationResult';
import { Icon } from './Icon';
import '../styles/projects.css';
import '../styles/operations.css';

interface OperationsProps {
  projects: Project[];
  generation: number;
  refreshing?: boolean;
  onOpen: OpenSource;
  onRefresh: () => void;
}

/**
 * 项目运作（规范第 3、9 节）：网格上方是项目选择；网格第一格是项目名与结构声明来源，
 * 读取中与错误也落在这一格；其后是各块证据；最后一格是「重新核验目录」。
 * 项目选择不随内容重挂，换项目时键盘焦点留在选择框上。
 */
export function ProjectOperations({
  projects,
  generation,
  refreshing = false,
  onOpen,
  onRefresh,
}: OperationsProps) {
  const { location, select } = useViewLocation();
  const selection = location.project ?? '';
  const project =
    projects.find((item) => item.root === selection) ?? projects[0];
  if (!project)
    return (
      <div className="operation-empty" role="status">
        <EmptyState title="当前快照没有可查看运作结构的项目。" />
      </div>
    );
  return (
    <div className="project-operations">
      <div className="operation-toolbar">
        <label
          className="operation-toolbar__label"
          htmlFor="operations-project"
        >
          项目
        </label>
        <span className="project-field project-field--select">
          <select
            id="operations-project"
            value={project.root}
            onChange={(event) => select({ project: event.target.value })}
          >
            {projects.map((item) => (
              <option key={item.id} value={item.root}>
                {item.name}
              </option>
            ))}
          </select>
          <Icon name="chevron" />
        </span>
        <span className="operation-toolbar__root">{project.root}</span>
      </div>
      {refreshing ? (
        <GridFrame className="operation-grid">
          <Cell span={12} className="operation-heading">
            <CellHeader title={project.name} />
            <Skeleton label="正在刷新工作台，项目运作证据将在刷新后重新读取…" />
          </Cell>
        </GridFrame>
      ) : (
        <ProjectOperationContent
          key={`${project.root}:${generation}`}
          project={project}
          generation={generation}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      )}
    </div>
  );
}

function ProjectOperationContent({
  project,
  generation,
  onOpen,
  onRefresh,
}: {
  project: Project;
  generation: number;
  onOpen: OpenSource;
  onRefresh: () => void;
}) {
  const { result, loading, error, stale, retry } = useProjectOperations(
    project.root,
    generation,
  );
  const data = !error && !loading ? result : null;
  const declaration = data?.declaration_source ?? null;
  return (
    <GridFrame className="operation-grid" aria-busy={loading}>
      <Cell span={12} className="operation-heading">
        <CellHeader
          title={project.name}
          action={
            declaration ? (
              <Button size="sm" onClick={() => onOpen(declaration)}>
                <Icon name="document" />
                查看结构声明
              </Button>
            ) : undefined
          }
        />
        {data ? (
          <p className="operation-heading__source">
            <span className="operation-heading__source-label">结构声明</span>
            <span>
              {declaration
                ? `${declaration.path} · 第 ${declaration.line} 行`
                : '暂无可定位的结构声明来源'}
            </span>
            <span>文档快照 {data.generation}</span>
          </p>
        ) : null}
        {error ? (
          <ErrorState
            title={stale ? '快照已更新，请刷新工作台' : '无法读取项目运作'}
            onRetry={stale ? onRefresh : undefined}
            retryLabel="刷新工作台"
          >
            <p>{error}</p>
          </ErrorState>
        ) : loading || !result ? (
          <Skeleton label="正在读取项目声明并核验目录…" />
        ) : null}
      </Cell>
      {data ? <OperationResult data={data} onOpen={onOpen} /> : null}
      {/* 这一格始终是网格的最后一个孩子：等待、成功、错误之间按钮是同一个节点，焦点不丢 */}
      <Cell span={12} className="operation-refresh">
        <p className="operation-refresh__note">
          目录发生变化后可重新核验；修改结构声明后请刷新工作台。
        </p>
        <Button
          aria-disabled={loading || stale}
          onClick={() => {
            if (!loading && !stale) retry();
          }}
        >
          <Icon name="refresh" />
          {error && !stale ? '重试' : '重新核验目录'}
        </Button>
      </Cell>
    </GridFrame>
  );
}
