import { useId, useState } from 'react';
import { useViewLocation } from '../useViewLocation';
import { motion } from 'motion/react';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
import type { OpenSource, Project } from '../types';
import { useSearch } from '../useSearch';
import { QueryResults } from './QueryResults';
import { SearchControls } from './SearchControls';
import { FileTree } from './FileTree';

export function Documents({
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
  const { location, select } = useViewLocation();
  const mode = location.panel === 'tree' ? 'tree' : 'search';
  // 从 URL 恢复到文件树时，树也要跟着一起挂上来，否则回来只剩一个空面板。
  const [treeVisited, setTreeVisited] = useState(mode === 'tree');
  const selectionId = useId();
  const reduceMotion = useReducedMotionPreference();
  return (
    <>
      <div className="document-modes" role="group" aria-label="文档浏览方式">
        <button
          aria-pressed={mode === 'search'}
          onClick={() => select({ panel: null })}
        >
          {mode === 'search' ? (
            <motion.span
              key={reduceMotion ? 'reduced' : 'animated'}
              className="document-mode-selected"
              aria-hidden="true"
              layoutId={reduceMotion ? undefined : selectionId}
              initial={false}
              transition={{ type: 'spring', stiffness: 500, damping: 45 }}
            />
          ) : null}
          搜索
        </button>
        <button
          aria-pressed={mode === 'tree'}
          onClick={() => {
            setTreeVisited(true);
            select({ panel: 'tree' });
          }}
        >
          {mode === 'tree' ? (
            <motion.span
              key={reduceMotion ? 'reduced' : 'animated'}
              className="document-mode-selected"
              aria-hidden="true"
              layoutId={reduceMotion ? undefined : selectionId}
              initial={false}
              transition={{ type: 'spring', stiffness: 500, damping: 45 }}
            />
          ) : null}
          文件树
        </button>
      </div>
      <div className="document-panel" hidden={mode !== 'search'}>
        <DocumentSearch
          projects={projects}
          generation={generation}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      </div>
      {treeVisited ? (
        <div className="document-panel" hidden={mode !== 'tree'}>
          <FileTree
            key={generation}
            generation={generation}
            onOpen={onOpen}
            onRefresh={onRefresh}
          />
        </div>
      ) : null}
    </>
  );
}

function DocumentSearch({
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
  const [query, setQuery] = useState('');
  const [project, setProject] = useState('');
  const search = useSearch(
    query,
    query.trim() ? 'full_text' : 'path',
    project,
    generation,
  );
  return (
    <section aria-label="搜索文档">
      <SearchControls
        query={query}
        onQuery={setQuery}
        project={project}
        onProject={setProject}
        projects={projects}
      />
      <QueryResults
        search={search}
        onOpen={onOpen}
        onRefresh={onRefresh}
        emptyMessage={
          query.trim() || project
            ? '没有匹配的文档，请调整搜索词或项目范围。'
            : '当前快照没有可读取的文档。'
        }
      />
    </section>
  );
}
