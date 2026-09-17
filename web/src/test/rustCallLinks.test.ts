import { expect, it } from 'vitest';
import { rustUnresolvedReasons } from '../rustCallLinks';
import { validateRustCallSites } from '../validateRustCallSites';
import { rustData } from './rustCallFixtures';
import { rustAnalysis, rustLinks } from './rustCallLinksFixtures';

it('接受同文件定义、多引用和递归；不把批标识当作前端已重算的内容哈希', () => {
  const data = rustAnalysis();
  expect(validateRustCallSites(data)).toBe(data);
  data.links = rustLinks({ batch_id: '0123456789abcdef'.repeat(4) });
  expect(validateRustCallSites(data).links?.batch_id).toBe(data.links.batch_id);
});

it('缺links兼容T044，ready存在但为null或损坏不可吞掉', () => {
  expect(validateRustCallSites(rustData()).links).toBeUndefined();
  for (const links of [null, false, [], {}, { version: 1 }, undefined]) {
    expect(() => validateRustCallSites({ ...rustAnalysis(), links })).toThrow(
      'Rust 词法关联',
    );
  }
});

it.each([
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
] as const)('%s兼容缺字段，present必须清空关联', (status) => {
  const data = rustData({ status, graph: null, observed_at: null });
  expect(validateRustCallSites(data).links).toBeUndefined();
  expect(validateRustCallSites({ ...data, links: null }).links).toBeNull();
  expect(() => validateRustCallSites({ ...data, links: rustLinks() })).toThrow(
    'Rust 词法关联',
  );
});

it.each([0, 2, '1', [1], null])('拒绝不支持的版本 %#', (version) => {
  expect(() =>
    validateRustCallSites({
      ...rustAnalysis(),
      links: { ...rustLinks(), version },
    }),
  ).toThrow();
});

it.each([
  '',
  'a'.repeat(63),
  'a'.repeat(65),
  'A'.repeat(64),
  'g'.repeat(64),
  ['a'.repeat(64)],
  null,
])('批标识必须为64个小写hex字符 %#', (batch_id) => {
  expect(() =>
    validateRustCallSites({
      ...rustAnalysis(),
      links: { ...rustLinks(), batch_id },
    }),
  ).toThrow();
});

it.each([
  'missing',
  'extra',
  'duplicate',
  'order',
  'unknown',
  'target',
  'target-kind',
  'cross-file',
  'linked-reason',
  'unresolved-target',
  'unresolved-reason',
  'array-status',
  'array-reason',
  'non-path-linked',
])('拒绝关联引用或状态损坏：%s', (kind) => {
  const data = rustAnalysis();
  const links = data.links!;
  const graph = data.graph!;
  if (kind === 'missing') links.associations.pop();
  if (kind === 'extra') links.associations.push(links.associations[0]);
  if (kind === 'duplicate') links.associations[1] = links.associations[0];
  if (kind === 'order') links.associations.reverse();
  if (kind === 'unknown') links.associations[0].call_id = 'f0:call:1:2';
  if (kind === 'target')
    Object.assign(links.associations[0], { target: 'f0:context:1:2' });
  if (kind === 'target-kind') graph.contexts[2].kind = 'module';
  if (kind === 'cross-file') {
    graph.files[1].bytes = 100;
    graph.contexts.push({
      id: 'f1:context:0:100',
      parent: 'f1:root',
      kind: 'function',
      name: '读取',
      source: { path: 'src/other.rs', line: 1, column: 1 },
    });
    Object.assign(links.associations[0], { target: 'f1:context:0:100' });
  }
  if (kind === 'linked-reason')
    Object.assign(links.associations[0], { reason: 'not_found' });
  if (kind === 'unresolved-target')
    Object.assign(links.associations[2], { target: graph.contexts[2].id });
  if (kind === 'unresolved-reason')
    Object.assign(links.associations[2], { reason: 'runtime' });
  if (kind === 'array-status')
    Object.assign(links.associations[0], { status: ['linked'] });
  if (kind === 'array-reason')
    Object.assign(links.associations[2], { reason: ['not_found'] });
  if (kind === 'non-path-linked') graph.calls[0].kind = 'method';
  expect(() => validateRustCallSites(data)).toThrow('Rust 词法关联');
});

it.each(Object.keys(rustUnresolvedReasons))(
  '接受闭集未解析原因%s并保留null目标',
  (reason) => {
    const data = rustAnalysis();
    Object.assign(data.links!.associations[2], { reason });
    expect(validateRustCallSites(data)).toBe(data);
  },
);

it('零调用对应空关联，不遗漏批标识', () => {
  const data = rustAnalysis();
  data.graph!.calls = [];
  data.links!.associations = [];
  expect(validateRustCallSites(data)).toBe(data);
});

it('拒绝异名和限定路径；原始标识及NFC等价名仍可关联', () => {
  const data = rustAnalysis();
  for (const name of ['different', '读取::读取', 'crate::读取']) {
    data.graph!.calls[0].name = name;
    expect(() => validateRustCallSites(data)).toThrow('Rust 词法关联');
  }
  data.graph!.contexts[2].name = 'é';
  for (const index of [0, 1, 4]) data.graph!.calls[index].name = 'r#e\u0301';
  expect(validateRustCallSites(data)).toBe(data);
});

it.each(['impl', 'trait', 'const', 'static', 'file'] as const)(
  '允许词法范围外%s上下文不能伪造linked',
  (kind) => {
    const data = rustAnalysis();
    data.graph!.calls[0].context = 'f0:root';
    if (kind !== 'file') {
      data.graph!.contexts[1].kind = kind;
      if (kind === 'impl') data.graph!.contexts[1].name = null;
      data.graph!.calls[0].context = data.graph!.contexts[1].id;
    }
    expect(() => validateRustCallSites(data)).toThrow('Rust 词法关联');
  },
);
