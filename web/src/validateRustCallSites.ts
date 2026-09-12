import type {
  RustCallGraph,
  RustCallSitesData,
  RustCallContext,
} from './rustCallSites';
import { rustCallKinds, rustContextKinds } from './rustCallSites';
import { validateRustCallLinks } from './rustCallLinks';
import {
  collisionFree,
  containsEvidence,
  evidenceId,
  integer,
  location,
  record,
  rustPath,
  sortedFiles,
  visibleText,
} from './rustSourceValidation';

function validGraph(value: unknown): value is RustCallGraph {
  if (
    !record(value) ||
    !Array.isArray(value.files) ||
    value.files.length < 1 ||
    value.files.length > 16 ||
    !value.files.every(
      (file: unknown) =>
        record(file) &&
        rustPath(file.path) &&
        integer(file.bytes, 65536) &&
        [
          'attributes',
          'conditional_attributes',
          'unexpanded_macros',
          'macro_definitions',
          'external_modules',
        ].every((key) => integer(file[key], 32768)) &&
        Number(file.conditional_attributes) <= Number(file.attributes),
    ) ||
    !Array.isArray(value.contexts) ||
    value.contexts.length > 256 ||
    !Array.isArray(value.calls) ||
    value.calls.length > 1024
  )
    return false;
  const graph = value as unknown as RustCallGraph;
  if (
    graph.files.reduce((sum, file) => sum + file.bytes, 0) > 512 * 1024 ||
    new Set(graph.files.map((file) => file.path)).size !== graph.files.length ||
    !collisionFree(graph.files) ||
    !sortedFiles(graph.files)
  )
    return false;
  const contexts = new Map<string, RustCallContext>();
  for (const item of value.contexts) {
    if (
      !record(item) ||
      typeof item.kind !== 'string' ||
      !Object.hasOwn(rustContextKinds, item.kind)
    )
      return false;
    const id = evidenceId(item.id, 'context', graph.files);
    if (!id || !location(item.source, id, graph.files)) return false;
    const nullable = ['file', 'impl', 'closure', 'async'].includes(item.kind);
    if (
      nullable
        ? item.name !== null
        : item.kind === 'const'
          ? item.name !== null && !visibleText(item.name, 256)
          : !visibleText(item.name, 256)
    )
      return false;
    if (item.kind === 'const' && item.name === '_') return false;
    if (
      id.root
        ? item.kind !== 'file' || item.parent !== null
        : item.kind === 'file' || typeof item.parent !== 'string'
    )
      return false;
    if (contexts.has(String(item.id))) return false;
    contexts.set(String(item.id), item as unknown as RustCallContext);
  }
  for (let index = 0; index < graph.files.length; index += 1) {
    if (!contexts.has(`f${index}:root`)) return false;
  }
  for (const context of contexts.values()) {
    const seen = new Set<string>();
    let current: RustCallContext | undefined = context;
    while (current) {
      if (seen.has(current.id)) return false;
      seen.add(current.id);
      if (current.parent === null) break;
      const parent: RustCallContext | undefined = contexts.get(current.parent);
      if (
        !parent ||
        parent.source.path !== context.source.path ||
        !containsEvidence(parent.id, current.id, 'context', graph.files)
      )
        return false;
      current = parent;
    }
  }
  const calls = new Set<string>();
  for (const item of value.calls) {
    if (
      !record(item) ||
      typeof item.kind !== 'string' ||
      !Object.hasOwn(rustCallKinds, item.kind)
    )
      return false;
    const id = evidenceId(item.id, 'call', graph.files);
    if (
      !id ||
      !location(item.source, id, graph.files) ||
      (item.kind === 'expression'
        ? item.name !== null
        : !visibleText(item.name, 256)) ||
      typeof item.context !== 'string' ||
      contexts.get(item.context)?.source.path !== item.source.path ||
      !containsEvidence(item.context, String(item.id), 'call', graph.files) ||
      calls.has(String(item.id))
    )
      return false;
    calls.add(String(item.id));
  }
  return graph.files.every(
    (file) =>
      graph.calls.filter(
        (call) => call.source.path === file.path && call.kind === 'macro',
      ).length === file.unexpanded_macros,
  );
}

export function validateRustCallSites(value: unknown): RustCallSitesData {
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
    value.diagnostics.every(
      (message: unknown) => typeof message === 'string',
    ) &&
    (value.status === 'ready'
      ? typeof value.observed_at === 'string' &&
        /^\d+$/.test(value.observed_at) &&
        integer(Number(value.observed_at), 8_640_000_000_000) &&
        validGraph(value.graph)
      : value.observed_at === null && value.graph === null);
  if (!valid)
    throw new Error(
      'Rust 调用位置数据格式、来源或覆盖范围不完整，请重试或检查服务。',
    );
  if (record(value) && 'links' in value) {
    if (value.status === 'ready')
      validateRustCallLinks(value.links, value.graph as RustCallGraph);
    else if (value.links !== null)
      throw new Error('Rust 词法关联失败状态必须清空本次关联证据。');
  }
  return value as unknown as RustCallSitesData;
}
