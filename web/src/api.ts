import { validateDocument, validateWorkbench } from './validate';
import { queryError, validateSearch } from './searchQuery';
import type { SearchMode } from './types';
import { validateTree } from './treeQuery';
import { ApiError, getJson } from './sessionHttp';
export { ApiError } from './sessionHttp';

export function getWorkbench(signal: AbortSignal, refresh = false) {
  return getJson<unknown>(
    `/api/workbench${refresh ? '?refresh=true' : ''}`,
    signal,
  ).then(validateWorkbench);
}

export function getDocument(
  path: string,
  generation: number,
  signal: AbortSignal,
) {
  const query = new URLSearchParams({ path, generation: String(generation) });
  return getJson<unknown>(`/api/document?${query}`, signal).then((value) => {
    const document = validateDocument(value);
    if (document.path !== path)
      throw new Error('原文路径与来源不一致，请刷新工作台。');
    return document;
  });
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : '读取失败，请重试。';
}

export function getSearch(
  query: string,
  mode: SearchMode,
  project: string,
  generation: number,
  signal: AbortSignal,
) {
  const invalid = queryError(query);
  if (invalid) return Promise.reject(new Error(invalid));
  const params = new URLSearchParams({
    q: query.trim(),
    mode,
    generation: String(generation),
  });
  if (project) params.set('project', project);
  return getJson<unknown>(`/api/search?${params}`, signal).then((value) => {
    const response = validateSearch(value);
    if (response.generation !== generation)
      throw new ApiError('搜索结果来自不同快照，请刷新工作台。', 409);
    return response;
  });
}

export function getTree(
  path: string,
  offset: number,
  generation: number,
  signal: AbortSignal,
) {
  const params = new URLSearchParams({
    path,
    offset: String(offset),
    generation: String(generation),
  });
  return getJson<unknown>(`/api/tree?${params}`, signal).then((value) => {
    const response = validateTree(value);
    if (response.generation !== generation)
      throw new ApiError('文件树来自不同快照，请刷新工作台。', 409);
    if (response.path !== path || response.offset !== offset)
      throw new Error('文件树位置与请求不一致，请重试。');
    return response;
  });
}
