import type { SourceLocation } from './types';
import { validateArchitecture } from './architecture';
import type { ArchitectureData } from './architecture';
import { validateCargoDependencies } from './cargoDependencies';
import type { CargoDependenciesData } from './cargoDependencies';

export interface OperationDirectory {
  path: string;
  label: string;
  source: SourceLocation | null;
}

export interface DirectoryDrift {
  path: string;
  status: 'present' | 'missing' | 'undocumented' | 'unverified';
  source: SourceLocation | null;
}

export interface ProjectOperationsData {
  generation: number;
  project: string;
  declaration_status: 'ready' | 'unavailable' | 'invalid';
  declaration_source: SourceLocation | null;
  directories: OperationDirectory[];
  comparison_parents: string[];
  diagnostics: string[];
  repository: {
    status: 'unbound' | 'ready' | 'unavailable';
    observed_at: string | null;
  };
  drift: DirectoryDrift[];
  architecture?: ArchitectureData;
  cargo_dependencies?: CargoDependenciesData;
}

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === 'string');
const source = (value: unknown): boolean =>
  value === null ||
  (record(value) &&
    typeof value.path === 'string' &&
    Number.isSafeInteger(value.line) &&
    Number(value.line) > 0);

function timestamp(value: unknown): boolean {
  return (
    value === null ||
    (typeof value === 'string' &&
      /^\d+$/.test(value) &&
      Number.isSafeInteger(Number(value)) &&
      Number(value) <= 8_640_000_000_000)
  );
}

export function validateProjectOperations(
  value: unknown,
): ProjectOperationsData {
  const valid =
    record(value) &&
    Number.isSafeInteger(value.generation) &&
    Number(value.generation) >= 0 &&
    typeof value.project === 'string' &&
    ['ready', 'unavailable', 'invalid'].includes(
      String(value.declaration_status),
    ) &&
    source(value.declaration_source) &&
    Array.isArray(value.comparison_parents) &&
    value.comparison_parents.length <= 64 &&
    strings(value.comparison_parents) &&
    strings(value.diagnostics) &&
    Array.isArray(value.directories) &&
    value.directories.length <= 256 &&
    value.directories.every(
      (item: unknown) =>
        record(item) &&
        typeof item.path === 'string' &&
        typeof item.label === 'string' &&
        source(item.source),
    ) &&
    new Set(value.directories.map((item) => item.path)).size ===
      value.directories.length &&
    record(value.repository) &&
    ['unbound', 'ready', 'unavailable'].includes(
      String(value.repository.status),
    ) &&
    timestamp(value.repository.observed_at) &&
    Array.isArray(value.drift) &&
    value.drift.length <= 1280 &&
    value.drift.every(
      (item: unknown) =>
        record(item) &&
        typeof item.path === 'string' &&
        ['present', 'missing', 'undocumented', 'unverified'].includes(
          String(item.status),
        ) &&
        source(item.source),
    ) &&
    new Set(value.drift.map((item) => item.path)).size === value.drift.length;
  if (!valid) throw new Error('项目运作数据格式不完整，请重试或检查服务。');
  if (record(value) && 'architecture' in value)
    validateArchitecture(value.architecture, String(value.project));
  if (record(value) && 'cargo_dependencies' in value)
    validateCargoDependencies(value.cargo_dependencies);
  return value as unknown as ProjectOperationsData;
}
