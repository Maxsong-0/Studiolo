import { expect, it } from 'vitest';
import { validateCargoDependencies } from '../cargoDependencies';
import { cargoData, cargoGraph } from './cargoFixtures';

it('接受64包256内部边及全部2048条声明的边界，额外一项即拒绝', () => {
  const graph = cargoGraph();
  graph.packages = Array.from({ length: 64 }, (_, index) => ({
    id: `pkg${index}`,
    name: `package${index}`,
    source: { path: `pkg${index}/Cargo.toml`, line: 1 },
  }));
  graph.dependencies = Array.from({ length: 256 }, (_, index) => ({
    ...graph.dependencies[0],
    from: 'pkg0',
    to: `pkg${index % 64}`,
    alias: `alias${index}`,
    source: { path: 'pkg0/Cargo.toml', line: 12 },
  }));
  graph.external_dependencies = 2048 - 256;
  expect(validateCargoDependencies(cargoData({ graph })).graph).toBe(graph);
  graph.external_dependencies += 1;
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  graph.external_dependencies = 0;
  graph.dependencies.push({ ...graph.dependencies[0], alias: 'extra' });
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  graph.dependencies = [];
  graph.packages.push({
    id: 'extra',
    name: 'extra',
    source: { path: 'extra/Cargo.toml', line: 1 },
  });
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
});

it('外部声明计数必须是非负整数，空包集合不能形成ready', () => {
  for (const external_dependencies of [-1, 0.5, 2049])
    expect(() =>
      validateCargoDependencies(
        cargoData({ graph: cargoGraph({ external_dependencies }) }),
      ),
    ).toThrow();
  expect(() =>
    validateCargoDependencies(
      cargoData({ graph: cargoGraph({ packages: [], dependencies: [] }) }),
    ),
  ).toThrow();
});

it('目标条件按UTF8字节检查256上限，名称和alias限制64 ASCII字节', () => {
  const graph = cargoGraph();
  graph.dependencies[0].target = '中'.repeat(85) + 'a';
  expect(validateCargoDependencies(cargoData({ graph })).status).toBe('ready');
  graph.dependencies[0].target += '中';
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  graph.dependencies[0].target = null;
  graph.packages[0].name = 'a'.repeat(65);
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
  graph.packages[0].name = 'app';
  graph.dependencies[0].alias = 'a'.repeat(65);
  expect(() => validateCargoDependencies(cargoData({ graph }))).toThrow();
});

it('16层和1024 UTF8字节目录在预算内仍可显示', () => {
  for (const id of [Array(16).fill('a').join('/'), '中'.repeat(341) + 'a']) {
    const graph = cargoGraph({
      dependencies: [],
      packages: [
        { id, name: 'app', source: { path: `${id}/Cargo.toml`, line: 1 } },
      ],
    });
    expect(validateCargoDependencies(cargoData({ graph })).status).toBe(
      'ready',
    );
  }
});
