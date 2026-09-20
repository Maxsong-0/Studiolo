import type { Freshness } from './freshness';

export interface SourceLocation {
  path: string;
  line: number;
}

export interface Sourced<T> {
  value: T | null;
  source: SourceLocation | null;
}

export interface Project {
  id: string;
  name: string;
  root: string;
  lifecycle: Sourced<string>;
  version: Sourced<string>;
  active_tasks: Sourced<number>;
  open_bugs: Sourced<number>;
  next_milestone: Sourced<string>;
  profile: string;
  source: SourceLocation | null;
  diagnostics: string[];
}

export interface Action {
  id: string;
  project_id: string;
  project_name: string;
  kind: string;
  title: string;
  source: SourceLocation;
}

export interface DocumentSummary {
  path: string;
  title: string;
}

/** 一个计数及其不可得的原因；`value` 为 null 表示不可得，绝不退化成 0。 */
export interface CountRollup {
  value: number | null;
  unavailable: string[];
}

/** 首屏计数；文档只报总数，逐篇清单走文件树分页与搜索。 */
export interface WorkbenchCounts {
  documents: CountRollup;
  projects: CountRollup;
  inbox: CountRollup;
  diagnostics: CountRollup;
  active_tasks: CountRollup;
  open_bugs: CountRollup;
}

export interface Workbench {
  generation: number;
  source_label: string;
  projects: Project[];
  inbox: Action[];
  counts: WorkbenchCounts;
  diagnostics: string[];
  freshness?: Freshness | null;
}

export interface SourceDocument {
  path: string;
  title: string;
  content: string;
  navigation: DocumentNavigationData;
}

export interface OutlineEntry {
  title: string;
  level: number;
  source: SourceLocation;
}

export interface OutgoingLink {
  label: string;
  raw: string;
  source: SourceLocation;
  status:
    | 'resolved'
    | 'ambiguous'
    | 'missing'
    | 'missing_heading'
    | 'ambiguous_heading';
  targets: { path: string; line: number; title: string }[];
}

export interface DocumentNavigationData {
  outline: OutlineEntry[];
  outgoing: OutgoingLink[];
  backlinks: { title: string; source: SourceLocation; context: string }[];
  truncated: boolean;
}

export type SearchMode = 'full_text' | 'path';

export interface SearchHit {
  path: string;
  title: string;
  line: number;
  snippet: string;
  kind: 'path' | 'content';
}

export interface SearchResponse {
  generation: number;
  hits: SearchHit[];
  truncated: boolean;
}

export type View =
  | 'overview'
  | 'inbox'
  | 'documents'
  | 'operations'
  | 'policy'
  | 'board'
  | 'execution';
export type OpenSource = (source: SourceLocation) => void;
