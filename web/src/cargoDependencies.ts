import type { SourceLocation } from './types';

export interface CargoPackage {
  id: string;
  name: string;
  source: SourceLocation;
}

export interface CargoDependency {
  from: string;
  to: string;
  alias: string;
  kind: 'normal' | 'dev' | 'build';
  target: string | null;
  optional: boolean;
  source: SourceLocation;
  inherited_from: SourceLocation | null;
}

export interface CargoDependencyGraph {
  packages: CargoPackage[];
  dependencies: CargoDependency[];
  external_dependencies: number;
}

export interface CargoDependenciesData {
  status:
    | 'disabled'
    | 'unbound'
    | 'unavailable'
    | 'invalid'
    | 'unsupported'
    | 'ready';
  observed_at: string | null;
  graph: CargoDependencyGraph | null;
  diagnostics: string[];
}

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const name = (value: unknown): value is string =>
  typeof value === 'string' && /^[a-z0-9_-]{1,64}$/i.test(value);
const encoder = new TextEncoder();
const text = (value: unknown, limit: number): value is string =>
  typeof value === 'string' &&
  value.trim().length > 0 &&
  value.length <= limit &&
  encoder.encode(value).length <= limit;
const control = (value: string): boolean =>
  [...value].some((character) => {
    const code = character.charCodeAt(0);
    return code < 32 || (code >= 127 && code <= 159);
  });
// 与 core::is_unicode_combining_mark 和 operations_parse::repository_path 对齐。
const forbiddenPathCharacters = (value: string): boolean =>
  [...value].some((character) => {
    const code = character.charCodeAt(0);
    return (
      (code >= 0x0300 && code <= 0x036f) ||
      (code >= 0x1ab0 && code <= 0x1aff) ||
      (code >= 0x1dc0 && code <= 0x1dff) ||
      (code >= 0x20d0 && code <= 0x20ff) ||
      (code >= 0xfe20 && code <= 0xfe2f) ||
      (code >= 0x200b && code <= 0x200f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2060 && code <= 0x206f) ||
      code === 0xfeff
    );
  });

function packageId(value: unknown): value is string {
  if (value === '.') return true;
  if (
    !text(value, 1024) ||
    value.normalize('NFC') !== value ||
    value.includes('\\') ||
    /^[a-z]:/i.test(value) ||
    /%(?:2f|5c|00|2e)/i.test(value) ||
    control(value) ||
    forbiddenPathCharacters(value)
  )
    return false;
  const parts = value.split('/');
  return (
    parts.length <= 16 &&
    parts.every((part) => part.length > 0 && !part.startsWith('.'))
  );
}

export const cargoManifestPath = (id: string) =>
  id === '.' ? 'Cargo.toml' : `${id}/Cargo.toml`;
const source = (value: unknown, path: string): boolean =>
  record(value) &&
  value.path === path &&
  Number.isSafeInteger(value.line) &&
  Number(value.line) > 0 &&
  Number(value.line) <= 65536;

function collisionFree(packages: CargoPackage[]): boolean {
  const prefixes = new Map<string, string>();
  for (const item of packages) {
    if (item.id === '.') continue;
    const segments = item.id.split('/');
    for (let index = 1; index <= segments.length; index += 1) {
      const path = segments.slice(0, index).join('/');
      const previous = prefixes.get(path.toLowerCase());
      if (previous !== undefined && previous !== path) return false;
      prefixes.set(path.toLowerCase(), path);
    }
  }
  return true;
}

function validGraph(value: unknown): value is CargoDependencyGraph {
  if (
    !record(value) ||
    !Array.isArray(value.packages) ||
    value.packages.length === 0 ||
    value.packages.length > 64 ||
    !value.packages.every(
      (item: unknown) =>
        record(item) &&
        packageId(item.id) &&
        name(item.name) &&
        source(item.source, cargoManifestPath(item.id)),
    ) ||
    !Array.isArray(value.dependencies) ||
    value.dependencies.length > 256 ||
    !Number.isSafeInteger(value.external_dependencies) ||
    Number(value.external_dependencies) < 0 ||
    Number(value.external_dependencies) + value.dependencies.length > 2048
  )
    return false;
  const packages = value.packages as CargoPackage[];
  const ids = new Set(packages.map((item) => item.id));
  if (
    ids.size !== packages.length ||
    new Set(packages.map((item) => item.id.toLowerCase())).size !==
      packages.length ||
    new Set(packages.map((item) => item.name)).size !== packages.length ||
    !collisionFree(packages)
  )
    return false;
  if (
    !value.dependencies.every(
      (item: unknown) =>
        record(item) &&
        typeof item.from === 'string' &&
        ids.has(item.from) &&
        typeof item.to === 'string' &&
        ids.has(item.to) &&
        name(item.alias) &&
        typeof item.kind === 'string' &&
        ['normal', 'dev', 'build'].includes(item.kind) &&
        typeof item.optional === 'boolean' &&
        (item.target === null ||
          (text(item.target, 256) &&
            !control(item.target) &&
            !/[\u2028\u2029]/.test(item.target))) &&
        source(item.source, cargoManifestPath(item.from)) &&
        (item.inherited_from === null ||
          source(item.inherited_from, 'Cargo.toml')),
    )
  )
    return false;
  const edges = value.dependencies as CargoDependency[];
  return (
    new Set(
      edges.map((item) =>
        JSON.stringify([
          item.from,
          item.to,
          item.alias,
          item.kind,
          item.target,
        ]),
      ),
    ).size === edges.length
  );
}

export function validateCargoDependencies(
  value: unknown,
): CargoDependenciesData {
  const valid =
    record(value) &&
    typeof value.status === 'string' &&
    [
      'disabled',
      'unbound',
      'unavailable',
      'invalid',
      'unsupported',
      'ready',
    ].includes(value.status) &&
    Array.isArray(value.diagnostics) &&
    value.diagnostics.every((item: unknown) => typeof item === 'string') &&
    (value.status === 'ready'
      ? typeof value.observed_at === 'string' &&
        /^\d+$/.test(value.observed_at) &&
        Number.isSafeInteger(Number(value.observed_at)) &&
        Number(value.observed_at) <= 8_640_000_000_000 &&
        validGraph(value.graph)
      : value.observed_at === null && value.graph === null);
  if (!valid)
    throw new Error('Cargo 清单依赖数据格式或来源不完整，请重试或检查服务。');
  return value as unknown as CargoDependenciesData;
}

export const cargoKinds = {
  normal: '普通依赖',
  dev: '开发依赖',
  build: '构建依赖',
};
