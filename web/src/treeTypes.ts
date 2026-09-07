export interface TreeEntry {
  path: string;
  name: string;
  kind: 'directory' | 'markdown' | 'attachment';
  size_bytes: string | null;
  child_count: number | null;
}

export interface TreeResponse {
  generation: number;
  path: string;
  offset: number;
  total: number;
  entries: TreeEntry[];
  next_offset: number | null;
}

export const TREE_PAGE_SIZE = 200;
