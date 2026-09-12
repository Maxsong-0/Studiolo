import { expect, it } from 'vitest';
import type { RustCallGraph } from '../rustCallSites';
import { validateRustCallSites } from '../validateRustCallSites';
import { rustData, rustFile, rustGraph } from './rustCallFixtures';

function filesOnly(paths: string[], bytes = 0): RustCallGraph {
  return {
    files: paths.map((path) => rustFile(path, bytes)),
    contexts: paths.map((path, index) => ({
      id: `f${index}:root`,
      parent: null,
      kind: 'file',
      name: null,
      source: { path, line: 1, column: 1 },
    })),
    calls: [],
  };
}

it.each([
  '../outside.rs',
  '/root.rs',
  'C:relative.rs',
  '1:relative.rs',
  'src/.hidden.rs',
  'src//a.rs',
  'src\\a.rs',
  'src/%2fa.rs',
  'src/%2ea.rs',
  'src/a.RS',
  `${'x/'.repeat(16)}a.rs`,
  `${'界'.repeat(341)}.rs`,
  'src/\u0300a.rs',
  'src/\u1ab0a.rs',
  'src/\u20d0a.rs',
  'src/\ufe20a.rs',
  'src/\ud800.rs',
])('拒绝不规范白名单路径 %#', (path) => {
  expect(() =>
    validateRustCallSites(rustData({ graph: filesOnly([path]) })),
  ).toThrow();
});

it.each([
  '\u0000',
  '\t',
  '\u0085',
  '\u00ad',
  '\u061c',
  '\u2028',
  '\u202e',
  '\u2066',
  '\u{110bd}',
  '\u{e0001}',
])('路径和名称均拒绝不可见控制与格式字符 %#', (character) => {
  expect(() =>
    validateRustCallSites(
      rustData({ graph: filesOnly([`src/${character}x.rs`]) }),
    ),
  ).toThrow();
  const graph = rustGraph();
  graph.calls[0].name = `read${character}`;
  expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
});

it.each([
  { paths: ['a.rs', 'a.rs'] },
  { paths: ['A/a.rs', 'a/b.rs'] },
  { paths: ['a.rs', 'a.rs/b.rs'] },
  { paths: ['z.rs', 'a.rs'] },
])('拒绝重复、前缀大小写冲突、文件目录冲突及未排序白名单 %#', ({ paths }) => {
  expect(() =>
    validateRustCallSites(rustData({ graph: filesOnly(paths) })),
  ).toThrow();
});

it('UTF-8字节排序支持中文和补充平面文件，不依赖JS UTF-16排序', () => {
  const graph = filesOnly(['é:a.rs', '\ue000.rs', '\u{10000}.rs']);
  expect(validateRustCallSites(rustData({ graph })).graph).toBe(graph);
});

it('空源码仍保留文件根，16个文件和512KiB边界有效', () => {
  const empty = filesOnly(['empty.rs']);
  expect(
    validateRustCallSites(rustData({ graph: empty })).graph?.calls,
  ).toEqual([]);
  const max = filesOnly(
    Array.from(
      { length: 16 },
      (_, index) => `${String(index).padStart(2, '0')}.rs`,
    ),
    32768,
  );
  expect(validateRustCallSites(rustData({ graph: max })).graph).toBe(max);
  max.files[0].bytes += 1;
  expect(() => validateRustCallSites(rustData({ graph: max }))).toThrow();
});

it('拒绝空白名单、17文件、单文件超限、过多上下文/位置与非整数计数', () => {
  for (const graph of [
    filesOnly([]),
    filesOnly(Array.from({ length: 17 }, (_, index) => `${index}.rs`)),
    filesOnly(['large.rs'], 65537),
    rustGraph({ contexts: Array(257).fill(rustGraph().contexts[0]) }),
    rustGraph({ calls: Array(1025).fill(rustGraph().calls[0]) }),
  ])
    expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
  const graph = rustGraph();
  graph.files[0].attributes = 1.5;
  expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
});

it('名称按UTF-8字节限制且不能为空，HTML字面量保持文本而非清洗', () => {
  const graph = rustGraph();
  for (const name of ['', ' ', '界'.repeat(86)]) {
    graph.calls[0].name = name;
    expect(() => validateRustCallSites(rustData({ graph }))).toThrow();
  }
  graph.calls[0].name = '界'.repeat(85);
  expect(validateRustCallSites(rustData({ graph })).graph).toBe(graph);
  graph.calls[0].name = '<img src=x onerror=alert(1)>';
  expect(validateRustCallSites(rustData({ graph })).graph?.calls[0].name).toBe(
    graph.calls[0].name,
  );
});
