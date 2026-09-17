import { useState } from 'react';
import type { OpenSource, Project } from '../types';
import { useProjectOperations } from '../useProjectOperations';
import { OperationResult } from './OperationResult';
import { Icon } from './Icon';

interface OperationsProps {
  projects: Project[];
  generation: number;
  refreshing?: boolean;
  onOpen: OpenSource;
  onRefresh: () => void;
}

export function ProjectOperations({
  projects,
  generation,
  refreshing = false,
  onOpen,
  onRefresh,
}: OperationsProps) {
  const [selection, setSelection] = useState('');
  const project =
    projects.find((item) => item.root === selection) ?? projects[0];
  if (!project)
    return (
      <p className="empty-state" role="status">
        当前快照没有可查看运作结构的项目。
      </p>
    );
  return (
    <div className="project-operations">
      <div className="operation-project-control">
        <label htmlFor="operations-project">项目</label>
        <div className="select-control">
          <select
            id="operations-project"
            value={project.root}
            onChange={(event) => setSelection(event.target.value)}
          >
            {projects.map((item) => (
              <option key={item.id} value={item.root}>
                {item.name}
              </option>
            ))}
          </select>
          <Icon name="chevron" />
        </div>
        <span className="operation-muted">{project.root}</span>
      </div>
      {refreshing ? (
        <p className="empty-state" role="status">
          正在刷新工作台，项目运作证据将在刷新后重新读取…
        </p>
      ) : (
        <ProjectOperationContent
          key={`${project.root}:${generation}`}
          project={project.root}
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
  project: string;
  generation: number;
  onOpen: OpenSource;
  onRefresh: () => void;
}) {
  const { result, loading, error, stale, retry } = useProjectOperations(
    project,
    generation,
  );
  return (
    <div className="operation-content" aria-busy={loading}>
      {error ? (
        <div className="operation-feedback" role="alert">
          <h2>{stale ? '快照已更新，请刷新工作台' : '无法读取项目运作'}</h2>
          <p>{error}</p>
          {stale ? (
            <button className="outline-button" onClick={onRefresh}>
              刷新工作台
            </button>
          ) : null}
        </div>
      ) : loading || !result ? (
        <p className="empty-state" role="status">
          正在读取项目声明并核验目录…
        </p>
      ) : (
        <OperationResult data={result} onOpen={onOpen} />
      )}
      <div className="operation-refresh">
        <p className="operation-muted">
          目录发生变化后可重新核验；修改结构声明后请刷新工作台。
        </p>
        <button
          className="outline-button"
          aria-disabled={loading || stale}
          onClick={() => {
            if (!loading && !stale) retry();
          }}
        >
          <Icon name="refresh" />
          {error && !stale ? '重试' : '重新核验目录'}
        </button>
      </div>
    </div>
  );
}
