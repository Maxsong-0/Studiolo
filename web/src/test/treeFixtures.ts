import type { TreeEntry, TreeResponse } from '../treeTypes';

export function treeEntry(
  path: string,
  kind: TreeEntry['kind'] = 'attachment',
  size = '9007199254740993',
): TreeEntry {
  return {
    path,
    name: path.split('/').at(-1)!,
    kind,
    size_bytes: kind === 'directory' ? null : size,
    child_count: kind === 'directory' ? 2 : null,
  };
}

export function treePage(
  path: string,
  entries: TreeEntry[],
  extra: Partial<TreeResponse> = {},
): TreeResponse {
  return {
    generation: 7,
    path,
    offset: 0,
    total: entries.length,
    entries,
    next_offset: null,
    ...extra,
  };
}
