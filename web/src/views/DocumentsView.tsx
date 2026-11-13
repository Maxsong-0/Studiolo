import { Documents } from '../components/Documents';
import type { OpenSource, Project } from '../types';
import { ViewHeader } from './ViewHeader';

/** 文档（A 轮薄包装）：面板 `search`（默认，记作 null）、`tree` 与 `changes` 由 Documents 自己读写。 */
export function DocumentsView({
  projects,
  generation,
  onOpen,
  onRefresh,
}: {
  projects: Project[];
  generation: number;
  onOpen: OpenSource;
  onRefresh: () => void;
}) {
  return (
    <div className="view view-documents">
      <ViewHeader
        title="文档"
        description="搜索文档、浏览目录与附件，或核对最近的内容变化。"
      />
      <Documents
        key={generation}
        projects={projects}
        generation={generation}
        onOpen={onOpen}
        onRefresh={onRefresh}
      />
    </div>
  );
}
