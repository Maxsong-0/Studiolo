import type { Project } from '../types';
import { Icon } from './Icon';

export function SearchControls({
  query,
  onQuery,
  project,
  onProject,
  projects,
  quick = false,
}: {
  query: string;
  onQuery: (query: string) => void;
  project: string;
  onProject: (project: string) => void;
  projects: Project[];
  quick?: boolean;
}) {
  return (
    <div
      className="document-search-controls"
      role="search"
      aria-label={quick ? '快速切换搜索' : '文档搜索'}
    >
      <label className="search-control document-search-input">
        <span className="sr-only">
          {quick ? '按完整路径快速查找' : '搜索文档正文、文件名或路径'}
        </span>
        <Icon name="search" />
        <input
          type="search"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder={
            quick ? '输入路径片段，以空格分隔' : '搜索文档正文、文件名或路径'
          }
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      <label className="select-control project-search-select">
        <span className="sr-only">
          {quick ? '快速切换项目范围' : '文档项目范围'}
        </span>
        <select
          value={project}
          onChange={(event) => onProject(event.target.value)}
        >
          <option value="">全部项目</option>
          {projects.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        <Icon name="chevron" />
      </label>
    </div>
  );
}
