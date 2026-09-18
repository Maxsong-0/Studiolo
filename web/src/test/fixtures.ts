import type {
  CountRollup,
  DocumentNavigationData,
  DocumentSummary,
  Project,
  Sourced,
  Workbench,
  WorkbenchCounts,
} from '../types';

export const emptyNavigation: DocumentNavigationData = {
  outline: [],
  outgoing: [],
  backlinks: [],
  truncated: false,
};

const source = { path: '项目/星舟/00-总览/INDEX.md', line: 3 };
const sourced = <T>(value: T | null): Sourced<T> => ({ value, source });

export const project: Project = {
  id: 'xingzhou',
  name: '星舟',
  root: '项目/星舟',
  lifecycle: sourced('活跃'),
  version: sourced('M1'),
  active_tasks: sourced(2),
  open_bugs: sourced(0),
  next_milestone: sourced('只读工作台'),
  profile: 'standard',
  source,
  diagnostics: [],
};

/** 首屏只报计数，逐篇清单由文件树与搜索各自的入口给出；这里保留一份供那些入口取用。 */
export const documents: DocumentSummary[] = [
  { path: source.path, title: '星舟总览' },
];

const known = (value: number): CountRollup => ({ value, unavailable: [] });

function counts(documentCount: number): WorkbenchCounts {
  return {
    documents: known(documentCount),
    projects: known(3),
    inbox: known(1),
    diagnostics: known(0),
    active_tasks: { value: null, unavailable: ['灵感盒'] },
    open_bugs: { value: null, unavailable: ['灵感盒'] },
  };
}

export const fixture: Workbench = {
  generation: 7,
  source_label: '测试只读库',
  projects: [
    project,
    {
      ...project,
      id: 'archive',
      name: '资料夹',
      root: '项目/资料夹',
      lifecycle: sourced('静态资料'),
      active_tasks: sourced(0),
    },
    {
      ...project,
      id: 'inbox',
      name: '灵感盒',
      root: '项目/灵感盒',
      lifecycle: { value: null, source: null },
      version: { value: null, source: null },
      active_tasks: { value: null, source: null },
      open_bugs: { value: null, source: null },
      next_milestone: { value: null, source: null },
      source: null,
      profile: 'unclassified',
    },
  ],
  inbox: [
    {
      id: 'action-1',
      project_id: 'xingzhou',
      project_name: '星舟',
      kind: '待验收',
      title: '验收只读工作台',
      source,
    },
  ],
  counts: counts(documents.length),
  diagnostics: [],
};

export function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

export function defaultApiResponse(
  input: RequestInfo | URL,
  snapshot = fixture,
  hits = documents,
) {
  return String(input).startsWith('/api/search')
    ? jsonResponse({
        generation: snapshot.generation,
        hits: hits.map((document) => ({
          ...document,
          line: 1,
          snippet: '',
          kind: 'path',
        })),
        truncated: false,
      })
    : jsonResponse(snapshot);
}
