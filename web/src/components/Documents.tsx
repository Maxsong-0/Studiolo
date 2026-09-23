import { useId, useState } from 'react';
import { useViewLocation } from '../useViewLocation';
import type { OpenSource, Project } from '../types';
import { useSearch } from '../useSearch';
import { Cell, CellHeader, GridFrame, Tabs } from '../ui';
import { QueryResults } from './QueryResults';
import { SearchControls } from './SearchControls';
import { FileTree } from './FileTree';
import '../styles/search.css';

const panels = [
  { id: 'search', label: '搜索' },
  { id: 'tree', label: '文件树' },
] as const;

/**
 * 文档（设计规范第 9 节）：「搜索 · 文件树」两个子标签，面板位置记在 URL 的 `panel`
 * （搜索是默认，记作 null）。两个面板切走只藏不卸：查询、项目范围、目录展开与页码原样保留，
 * 藏起来的面板不重复请求。阅读器是 App 挂的全屏覆盖层（DocumentDialog），不在这里。
 */
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
  // 文件树第一次露面才挂上（才读根目录）。不论怎么到的文件树——点标签、⌘K、从 URL 恢复、
  // 挂着时 hash 被改成 panel=tree——都算来过，否则文件树面板会空着。
  const [treeVisited, setTreeVisited] = useState(mode === 'tree');
  if (mode === 'tree' && !treeVisited) setTreeVisited(true);
  const id = useId();
  const panel = (name: (typeof panels)[number]['id']) => ({
    role: 'tabpanel',
    id: `${id}-panel-${name}`,
    'aria-labelledby': `${id}-tab-${name}`,
    className: 'document-panel',
    hidden: mode !== name,
  });
  return (
    <div className="documents">
      <div className="documents-bar">
        <Tabs
          items={panels}
          value={mode}
          onChange={(next) =>
            select({ panel: next === 'tree' ? 'tree' : null })
          }
          ariaLabel="文档浏览方式"
          idPrefix={id}
        />
      </div>
      <div {...panel('search')}>
        <DocumentSearch
          projects={projects}
          generation={generation}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      </div>
      <div {...panel('tree')}>
        {treeVisited ? (
          <FileTree
            key={generation}
            generation={generation}
            onOpen={onOpen}
            onRefresh={onRefresh}
          />
        ) : null}
      </div>
    </div>
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
  const fullText = Boolean(query.trim());
  const search = useSearch(
    query,
    fullText ? 'full_text' : 'path',
    project,
    generation,
  );
  // 条数只在这一轮结果读到时给出（来自接口）；读取中或出错时不显示数字。
  const result = search.result;
  return (
    <section aria-label="搜索文档">
      <GridFrame className="document-search">
        <Cell as="div" className="document-search__controls">
          <SearchControls
            query={query}
            onQuery={setQuery}
            project={project}
            onProject={setProject}
            projects={projects}
          />
        </Cell>
        <Cell as="div" flush className="document-results">
          <div className="document-results__head">
            <CellHeader
              title={fullText ? '搜索结果' : '文档列表'}
              meta={
                result
                  ? `${result.hits.length}${result.truncated ? '+' : ''} 条`
                  : undefined
              }
            />
          </div>
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
        </Cell>
      </GridFrame>
    </section>
  );
}
