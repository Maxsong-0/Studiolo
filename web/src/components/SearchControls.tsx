import { COMMAND_TARGETS } from '../commands/focusTarget';
import type { Project } from '../types';
import { Icon } from './Icon';
import '../styles/search.css';

/**
 * 文档页的搜索控件：描边输入框＋原生 select（项目范围）。
 * 输入框挂 ⌘K 的带路标记：「删除文件」只把人带到这里、聚焦这个框，确认仍在原界面完成。
 */
export function SearchControls({
  query,
  onQuery,
  project,
  onProject,
  projects,
}: {
  query: string;
  onQuery: (query: string) => void;
  project: string;
  onProject: (project: string) => void;
  projects: Project[];
}) {
  return (
    <div
      className="document-search-controls"
      role="search"
      aria-label="文档搜索"
    >
      <label className="search-control document-search-input">
        <span className="sr-only">搜索文档正文、文件名或路径</span>
        <Icon name="search" />
        <input
          type="search"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder="搜索文档正文、文件名或路径"
          autoComplete="off"
          spellCheck={false}
          data-command-target={COMMAND_TARGETS.documentsSearch}
        />
      </label>
      <label className="select-control project-search-select">
        <span className="sr-only">文档项目范围</span>
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
