import type { SourceDocument, Workbench } from './types';
import { validNavigation } from './validateNavigation';

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  value !== null && typeof value === 'object';
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === 'string');
const source = (value: unknown): boolean =>
  record(value) &&
  typeof value.path === 'string' &&
  Number.isSafeInteger(value.line) &&
  Number(value.line) > 0;

function sourced(value: unknown, kind: 'string' | 'number'): boolean {
  return (
    record(value) &&
    (value.value === null ||
      (typeof value.value === kind &&
        (kind !== 'number' ||
          (Number.isSafeInteger(value.value) && Number(value.value) >= 0)))) &&
    (value.source === null || source(value.source))
  );
}

export function validateWorkbench(value: unknown): Workbench {
  const valid =
    record(value) &&
    Number.isSafeInteger(value.generation) &&
    Number(value.generation) >= 0 &&
    typeof value.source_label === 'string' &&
    strings(value.diagnostics) &&
    Array.isArray(value.projects) &&
    value.projects.every(
      (project: unknown) =>
        record(project) &&
        ['id', 'name', 'root', 'profile'].every(
          (field) => typeof project[field] === 'string',
        ) &&
        sourced(project.lifecycle, 'string') &&
        sourced(project.version, 'string') &&
        sourced(project.active_tasks, 'number') &&
        sourced(project.open_bugs, 'number') &&
        sourced(project.next_milestone, 'string') &&
        strings(project.diagnostics) &&
        (project.source === null || source(project.source)),
    ) &&
    Array.isArray(value.inbox) &&
    value.inbox.every(
      (action: unknown) =>
        record(action) &&
        ['id', 'project_id', 'project_name', 'kind', 'title'].every(
          (field) => typeof action[field] === 'string',
        ) &&
        source(action.source),
    ) &&
    Array.isArray(value.documents) &&
    value.documents.every(
      (document: unknown) =>
        record(document) &&
        typeof document.path === 'string' &&
        typeof document.title === 'string',
    );
  if (!valid) throw new Error('工作台数据格式不完整，请刷新或检查服务。');
  return value as unknown as Workbench;
}

export function validateDocument(value: unknown): SourceDocument {
  if (
    !record(value) ||
    !['path', 'title', 'content'].every(
      (field) => typeof value[field] === 'string',
    ) ||
    !validNavigation(value.navigation)
  ) {
    throw new Error('原文数据格式不完整，请重试。');
  }
  return value as unknown as SourceDocument;
}
