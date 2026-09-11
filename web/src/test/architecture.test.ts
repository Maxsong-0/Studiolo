import { expect, it } from 'vitest';
import { validateArchitecture } from '../architecture';
import { validateProjectOperations } from '../projectOperations';
import {
  architectureData,
  architectureLocation,
  architectureProject,
} from './architectureFixtures';
import { operationsData } from './operationsFixtures';

it('接受循环、自环和设计路径重复节点，保留原文字面值', () => {
  const data = architectureData();
  data.nodes[0].label = '<b>检索接口</b>';
  expect(validateArchitecture(data, architectureProject)).toBe(data);
});

it('旧响应缺架构字段仍有效，但字段存在且损坏必须拒绝', () => {
  expect(
    validateProjectOperations(operationsData()).architecture,
  ).toBeUndefined();
  for (const architecture of [null, {}, false, { status: 'ready' }]) {
    expect(() =>
      validateProjectOperations({ ...operationsData(), architecture }),
    ).toThrow('架构关系');
  }
});

it.each(['node-source', 'relation-source', 'step-source', 'top-source'])(
  '%s不能引用其他项目或其他文档',
  (kind) => {
    const data = architectureData();
    const source = { path: '项目/其他/01-框架/项目运作.md', line: 1 };
    if (kind === 'node-source') data.nodes[0].source = source;
    if (kind === 'relation-source') data.relations[0].source = source;
    if (kind === 'step-source') data.flows[0].steps[0].source = source;
    if (kind === 'top-source') data.source = source;
    expect(() => validateArchitecture(data, architectureProject)).toThrow(
      '架构关系',
    );
  },
);

it.each([
  'duplicate-node',
  'duplicate-relation',
  'duplicate-flow',
  'unknown-endpoint',
  'unknown-step',
  'invalid-order',
  'missing-source',
])('拒绝损坏引用、顺序和来源：%s', (kind) => {
  const data = architectureData();
  if (kind === 'duplicate-node') data.nodes.push(data.nodes[0]);
  if (kind === 'duplicate-relation') data.relations.push(data.relations[0]);
  if (kind === 'duplicate-flow') data.flows.push(data.flows[0]);
  if (kind === 'unknown-endpoint') data.relations[0].to = 'missing';
  if (kind === 'unknown-step') data.flows[0].steps[0].node = 'missing';
  if (kind === 'invalid-order') data.flows[0].steps[1].order = 3;
  if (kind === 'missing-source')
    data.nodes[0].source = { ...architectureLocation(1), line: 0 };
  expect(() => validateArchitecture(data, architectureProject)).toThrow(
    '架构关系',
  );
});

it.each(['Upper', 'with space', '中文', 'a'.repeat(49)])(
  '拒绝不符合ID契约的节点%s',
  (id) => {
    const data = architectureData();
    data.nodes[0].id = id;
    expect(() => validateArchitecture(data, architectureProject)).toThrow(
      '架构关系',
    );
  },
);

it.each(['label', 'description', 'directory'] as const)(
  '空%s不能进入架构视图',
  (field) => {
    const data = architectureData();
    data.nodes[0][field] = '';
    expect(() => validateArchitecture(data, architectureProject)).toThrow(
      '架构关系',
    );
  },
);

it('不可用状态不能夹带部分图，ready必须有节点和声明来源', () => {
  for (const override of [
    { status: 'invalid' as const },
    { nodes: [] },
    { source: null },
  ]) {
    expect(() =>
      validateArchitecture(architectureData(override), architectureProject),
    ).toThrow('架构关系');
  }
});

it.each([
  '../outside',
  '/absolute',
  'src/.hidden',
  'src//core',
  'src\\core',
  '-',
])('拒绝非规范目录关联%s', (directory) => {
  const data = architectureData();
  data.nodes[0].directory = directory;
  expect(() => validateArchitecture(data, architectureProject)).toThrow(
    '架构关系',
  );
});
