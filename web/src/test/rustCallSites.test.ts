import { expect, it } from 'vitest';
import { validateRustCallSites } from '../validateRustCallSites';
import { validateProjectOperations } from '../projectOperations';
import { rustData, rustGraph } from './rustCallFixtures';
import { operationsData } from './operationsFixtures';

it('接受中文、匿名闭包、原标识路径、表达式和空文件，并保留旧字段缺失降级', () => {
  const data = rustData();
  expect(validateRustCallSites(data)).toBe(data);
  expect(
    validateProjectOperations(operationsData()).rust_call_sites,
  ).toBeUndefined();
  const graph = rustGraph();
  graph.contexts = [
    graph.contexts[2],
    graph.contexts[1],
    graph.contexts[0],
    graph.contexts[3],
  ];
  expect(validateRustCallSites(rustData({ graph })).graph).toBe(graph);
});

it.each([null, false, [], {}, { status: 'ready' }])(
  '拒绝存在但损坏的字段 %#',
  (rust_call_sites) => {
    expect(() =>
      validateProjectOperations({ ...operationsData(), rust_call_sites }),
    ).toThrow('Rust 调用位置');
  },
);

it.each([
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
] as const)('%s原子清空图与时间', (status) => {
  expect(() => validateRustCallSites(rustData({ status }))).toThrow();
  expect(() =>
    validateRustCallSites(rustData({ status, graph: null })),
  ).toThrow();
  expect(() =>
    validateRustCallSites(rustData({ status, observed_at: null })),
  ).toThrow();
  expect(
    validateRustCallSites(rustData({ status, graph: null, observed_at: null }))
      .status,
  ).toBe(status);
});

it.each([
  'ready',
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
])('拒绝数组状态%s', (status) => {
  expect(() =>
    validateRustCallSites({ ...rustData(), status: [status] }),
  ).toThrow();
});

it.each([
  'file',
  'module',
  'impl',
  'trait',
  'function',
  'method',
  'closure',
  'async',
  'const',
  'static',
])('上下文种类%s不能由数组转字符串', (kind) => {
  const graph = rustGraph();
  Object.assign(graph.contexts[1], { kind: [kind] });
  expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
});

it.each(['path', 'method', 'expression', 'macro'])(
  '调用种类%s不能由数组转字符串',
  (kind) => {
    const graph = rustGraph();
    Object.assign(graph.calls[0], { kind: [kind] });
    expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
  },
);

it.each([
  'root-missing',
  'root-name',
  'root-source',
  'wrong-file-id',
  'duplicate-context',
  'missing-parent',
  'parent-cycle',
  'cross-file-parent',
  'parent-span',
  'call-span',
  'call-context',
  'duplicate-call',
  'source-path',
  'source-line',
  'source-column',
  'source-impossible',
  'id-leading-zero',
  'id-range',
  'expression-name',
  'path-null',
  'closure-name',
  'function-null',
  'macro-count',
  'conditional-count',
])('拒绝不完整的词法与来源证据：%s', (kind) => {
  const graph = rustGraph();
  if (kind === 'root-missing') graph.contexts.shift();
  if (kind === 'root-name') graph.contexts[0].name = 'root';
  if (kind === 'root-source') graph.contexts[0].source.column = 2;
  if (kind === 'wrong-file-id') graph.calls[0].id = 'f1:call:30:40';
  if (kind === 'duplicate-context') graph.contexts.push(graph.contexts[0]);
  if (kind === 'missing-parent') graph.contexts[1].parent = 'f0:context:1:2';
  if (kind === 'parent-cycle') graph.contexts[1].parent = graph.contexts[2].id;
  if (kind === 'cross-file-parent') graph.contexts[1].parent = 'f1:root';
  if (kind === 'parent-span') {
    graph.contexts[2].id = 'f0:context:70:195';
    graph.calls[2].context = graph.contexts[2].id;
  }
  if (kind === 'call-span') graph.calls[0].context = graph.contexts[2].id;
  if (kind === 'call-context') graph.calls[0].context = 'f1:root';
  if (kind === 'duplicate-call') graph.calls.push(graph.calls[0]);
  if (kind === 'source-path') graph.calls[0].source.path = '../secret.rs';
  if (kind === 'source-line') graph.calls[0].source.line = 0;
  if (kind === 'source-column') graph.calls[0].source.column = 0;
  if (kind === 'source-impossible')
    Object.assign(graph.calls[0].source, { line: 30, column: 30 });
  if (kind === 'id-leading-zero') graph.calls[0].id = 'f00:call:30:40';
  if (kind === 'id-range') graph.calls[0].id = 'f0:call:30:201';
  if (kind === 'expression-name') graph.calls[2].name = 'guessed_target';
  if (kind === 'path-null') graph.calls[0].name = null;
  if (kind === 'closure-name') graph.contexts[2].name = 'invented';
  if (kind === 'function-null') graph.contexts[1].name = null;
  if (kind === 'macro-count') graph.files[0].unexpanded_macros = 0;
  if (kind === 'conditional-count') graph.files[0].conditional_attributes = 4;
  expect(() => validateRustCallSites(rustData({ graph }))).toThrow(
    'Rust 调用位置',
  );
});

it('匿名const、impl、async和具名const均按kind保留名称契约', () => {
  for (const kind of ['const', 'impl', 'async'] as const) {
    const graph = rustGraph();
    Object.assign(graph.contexts[2], { kind, name: null });
    expect(validateRustCallSites(rustData({ graph })).graph).toBe(graph);
  }
  const graph = rustGraph();
  Object.assign(graph.contexts[2], { kind: 'const', name: 'VALUE' });
  expect(validateRustCallSites(rustData({ graph })).graph).toBe(graph);
  graph.contexts[2].name = '_';
  expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
});

it.each([null, '', 'NaN', '-1', '8640000000001', '9007199254740992'])(
  'ready拒绝无效观测时间 %#',
  (observed_at) => {
    expect(() => validateRustCallSites(rustData({ observed_at }))).toThrow();
  },
);
