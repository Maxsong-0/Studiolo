import type {
  DocumentNavigationData,
  Project,
  Sourced,
  Workbench,
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
  documents: [{ path: source.path, title: '星舟总览' }],
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
) {
  return String(input).startsWith('/api/search')
    ? jsonResponse({
        generation: snapshot.generation,
        hits: snapshot.documents.map((document) => ({
          ...document,
          line: 1,
          snippet: '',
          kind: 'path',
        })),
        truncated: false,
      })
    : jsonResponse(snapshot);
}
