import { expect, it } from 'vitest';
import { validateCargoDependencies } from '../cargoDependencies';
import { validateProjectOperations } from '../projectOperations';
import { cargoData, cargoGraph } from './cargoFixtures';
import { operationsData } from './operationsFixtures';

it('接受循环自环、别名、根包及继承源，旧响应缺字段不破坏工作台', () => {
  const data = cargoData();
  expect(validateCargoDependencies(data)).toBe(data);
  expect(
    validateProjectOperations(operationsData()).cargo_dependencies,
  ).toBeUndefined();
});

it.each([
  'ready',
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
])('拒绝可被字符串化的数组状态%s', (status) => {
  expect(() =>
    validateCargoDependencies({
      status: [status],
      graph: null,
      observed_at: null,
      diagnostics: [],
    }),
  ).toThrow('Cargo 清单依赖');
});

it.each(['normal', 'dev', 'build'])(
  '拒绝可被字符串化的数组依赖种类%s',
  (kind) => {
    const graph = cargoGraph();
    Object.assign(graph.dependencies[0], { kind: [kind] });
    expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow(
      'Cargo 清单依赖',
    );
  },
);

it.each([null, false, {}, { status: 'ready' }])(
  '存在但损坏的Cargo字段不能静默降级 %#',
  (cargo_dependencies) => {
    expect(() =>
      validateProjectOperations({ ...operationsData(), cargo_dependencies }),
    ).toThrow('Cargo 清单依赖');
  },
);

it.each([
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
] as const)('%s不得夹带旧图或旧观测时间', (status) => {
  expect(() => validateCargoDependencies(cargoData({ status }))).toThrow();
  expect(() =>
    validateCargoDependencies(cargoData({ status, graph: null })),
  ).toThrow();
  expect(
    validateCargoDependencies(
      cargoData({ status, graph: null, observed_at: null }),
    ).status,
  ).toBe(status);
});

it.each([
  'duplicate-id',
  'case-collision',
  'duplicate-name',
  'unknown-target',
  'duplicate-edge',
  'package-source',
  'edge-source',
  'inherit-source',
  'source-line',
  'optional',
  'name',
])('拒绝损坏清单身份与来源：%s', (kind) => {
  const graph = cargoGraph();
  if (kind === 'duplicate-id') graph.packages.push(graph.packages[0]);
  if (kind === 'case-collision')
    graph.packages.push({
      id: 'TOOLS',
      name: 'another',
      source: { path: 'TOOLS/Cargo.toml', line: 1 },
    });
  if (kind === 'duplicate-name')
    graph.packages[1].name = graph.packages[0].name;
  if (kind === 'unknown-target') graph.dependencies[0].to = 'unknown';
  if (kind === 'duplicate-edge') graph.dependencies.push(graph.dependencies[0]);
  if (kind === 'package-source')
    graph.packages[0].source.path = '项目/甲/笔记.md';
  if (kind === 'edge-source')
    graph.dependencies[0].source.path = 'tools/Cargo.toml';
  if (kind === 'inherit-source')
    graph.dependencies[0].inherited_from = {
      path: 'crates/core/Cargo.toml',
      line: 1,
    };
  if (kind === 'source-line') graph.packages[0].source.line = 0;
  if (kind === 'optional')
    Object.assign(graph.dependencies[0], { optional: 'false' });
  if (kind === 'name') graph.packages[0].name = '非法名称';
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow(
    'Cargo 清单依赖',
  );
});

it.each([
  '../outside',
  '/absolute',
  'src/.hidden',
  'src//core',
  'src\\core',
  'C:relative',
  Array(17).fill('a').join('/'),
  'a'.repeat(1025),
])('拒绝非规范或过深包目录 %#', (id) => {
  const graph = cargoGraph({
    dependencies: [],
    packages: [
      {
        id,
        name: 'package',
        source: { path: `${id}/Cargo.toml`, line: 1 },
      },
    ],
  });
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
});

it.each([
  '\u0300',
  '\u1ab0',
  '\u1dc0',
  '\u20d0',
  '\ufe20',
  '\u200b',
  '\u202e',
  '\u2066',
  '\ufeff',
])('包目录拒绝NFC仍保留的组合标记和格式控制字符 %#', (character) => {
  const id = `crates/${character}core`;
  expect(id.normalize('NFC')).toBe(id);
  const graph = cargoGraph({
    dependencies: [],
    packages: [
      {
        id,
        name: 'core',
        source: { path: `${id}/Cargo.toml`, line: 1 },
      },
    ],
  });
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow(
    'Cargo 清单依赖',
  );
});

it('拒绝祖先目录大小写碰撞及超出单清单字节预算的来源行', () => {
  const graph = cargoGraph({
    dependencies: [],
    packages: [
      { id: 'A/x', name: 'one', source: { path: 'A/x/Cargo.toml', line: 1 } },
      { id: 'a/y', name: 'two', source: { path: 'a/y/Cargo.toml', line: 1 } },
    ],
  });
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  const oversized = cargoGraph();
  oversized.packages[0].source.line = 65537;
  expect(() =>
    validateCargoDependencies(cargoData({ graph: oversized })),
  ).toThrow();
});

it.each(['cfg(a)\t', 'cfg(a)\u0000', 'cfg(a)\u0085', 'cfg(a)\n'])(
  '目标条件拒绝全部控制字符 %#',
  (target) => {
    const graph = cargoGraph();
    graph.dependencies[0].target = target;
    expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  },
);

it.each([null, '', 'NaN', '-1', '8640000000001', '9007199254740992'])(
  'ready拒绝不可信观测时间 %#',
  (observed_at) => {
    expect(() =>
      validateCargoDependencies(cargoData({ observed_at })),
    ).toThrow();
  },
);
