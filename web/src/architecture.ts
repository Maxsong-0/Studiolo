import type { SourceLocation } from './types';

export interface ArchitectureNode {
  id: string;
  label: string;
  kind: 'module' | 'interface' | 'external' | 'data';
  directory: string | null;
  description: string;
  source: SourceLocation;
}

export interface ArchitectureRelation {
  from: string;
  to: string;
  kind: 'dependency' | 'data_flow';
  label: string;
  source: SourceLocation;
}

export interface ArchitectureStep {
  order: number;
  node: string;
  description: string;
  source: SourceLocation;
}

export interface ArchitectureFlow {
  id: string;
  label: string;
  steps: ArchitectureStep[];
}

export interface ArchitectureData {
  status: 'ready' | 'unavailable' | 'invalid';
  source: SourceLocation | null;
  nodes: ArchitectureNode[];
  relations: ArchitectureRelation[];
  flows: ArchitectureFlow[];
  diagnostics: string[];
}

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string =>
  typeof value === 'string' && /^[a-z][a-z0-9_-]{0,47}$/.test(value);
const encoder = new TextEncoder();
function text(value: unknown, limit: number): value is string {
  return (
    typeof value === 'string' &&
    value.trim().length > 0 &&
    value.length <= limit &&
    encoder.encode(value).length <= limit
  );
}
const source = (value: unknown, path: string): boolean =>
  record(value) &&
  value.path === path &&
  Number.isSafeInteger(value.line) &&
  Number(value.line) > 0;

function directory(value: unknown): boolean {
  return (
    value === null ||
    (text(value, 1024) &&
      value !== '-' &&
      value.normalize('NFC') === value &&
      !value.includes('\\') &&
      ![...value].some(
        (character) =>
          character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
      ) &&
      !/^[a-z]:\//i.test(value) &&
      !/%(?:2f|5c|00|2e)/i.test(value) &&
      value
        .split('/')
        .every((part) => part.length > 0 && !part.startsWith('.')))
  );
}

export function validateArchitecture(
  value: unknown,
  project: string,
): ArchitectureData {
  const path = `${project}/01-框架/项目运作.md`;
  const valid =
    record(value) &&
    ['ready', 'unavailable', 'invalid'].includes(String(value.status)) &&
    (value.source === null || source(value.source, path)) &&
    Array.isArray(value.diagnostics) &&
    value.diagnostics.every((item: unknown) => typeof item === 'string') &&
    Array.isArray(value.nodes) &&
    value.nodes.length <= 64 &&
    value.nodes.every(
      (item: unknown) =>
        record(item) &&
        id(item.id) &&
        text(item.label, 256) &&
        ['module', 'interface', 'external', 'data'].includes(
          String(item.kind),
        ) &&
        directory(item.directory) &&
        text(item.description, 1024) &&
        source(item.source, path),
    ) &&
    Array.isArray(value.relations) &&
    value.relations.length <= 128 &&
    value.relations.every(
      (item: unknown) =>
        record(item) &&
        id(item.from) &&
        id(item.to) &&
        ['dependency', 'data_flow'].includes(String(item.kind)) &&
        text(item.label, 256) &&
        source(item.source, path),
    ) &&
    Array.isArray(value.flows) &&
    value.flows.length <= 8 &&
    value.flows.every(
      (item: unknown) =>
        record(item) &&
        id(item.id) &&
        text(item.label, 256) &&
        Array.isArray(item.steps) &&
        item.steps.length > 0 &&
        item.steps.length <= 32 &&
        item.steps.every(
          (step: unknown, index: number) =>
            record(step) &&
            step.order === index + 1 &&
            id(step.node) &&
            text(step.description, 1024) &&
            source(step.source, path),
        ),
    );
  if (!valid) throw new Error('架构关系数据格式不完整，请重试或检查服务。');
  const data = value as unknown as ArchitectureData;
  const nodes = new Set(data.nodes.map((node) => node.id));
  if (
    nodes.size !== data.nodes.length ||
    new Set(data.flows.map((flow) => flow.id)).size !== data.flows.length ||
    new Set(
      data.relations.map(
        (relation) => `${relation.from}:${relation.to}:${relation.kind}`,
      ),
    ).size !== data.relations.length ||
    data.relations.some(
      (relation) => !nodes.has(relation.from) || !nodes.has(relation.to),
    ) ||
    data.flows.some((flow) =>
      flow.steps.some((step) => !nodes.has(step.node)),
    ) ||
    (data.status !== 'ready' &&
      (data.nodes.length > 0 ||
        data.relations.length > 0 ||
        data.flows.length > 0)) ||
    (data.status === 'invalid' && data.source === null) ||
    (data.status === 'ready' &&
      (data.source === null || data.nodes.length === 0))
  )
    throw new Error('架构关系引用或来源不完整，请重试或检查服务。');
  return data;
}

export const nodeKinds = {
  module: '模块',
  interface: '接口',
  external: '外部',
  data: '数据',
};
export const relationKinds = {
  dependency: '依赖声明',
  data_flow: '数据流声明',
};
