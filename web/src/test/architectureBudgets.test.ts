import { expect, it } from 'vitest';
import { validateArchitecture } from '../architecture';
import {
  architectureData,
  architectureLocation,
  architectureProject,
} from './architectureFixtures';

it.each(['nodes', 'relations', 'flows', 'steps'] as const)(
  '超过%s预算时拒绝数据',
  (kind) => {
    const data = architectureData();
    if (kind === 'nodes')
      data.nodes = Array.from({ length: 65 }, (_, index) => ({
        ...data.nodes[0],
        id: `node${index}`,
      }));
    if (kind === 'relations')
      data.relations = Array.from({ length: 129 }, () => data.relations[0]);
    if (kind === 'flows')
      data.flows = Array.from({ length: 9 }, (_, index) => ({
        ...data.flows[0],
        id: `flow${index}`,
      }));
    if (kind === 'steps')
      data.flows[0].steps = Array.from({ length: 33 }, (_, index) => ({
        ...data.flows[0].steps[0],
        order: index + 1,
      }));
    expect(() => validateArchitecture(data, architectureProject)).toThrow(
      '架构关系',
    );
  },
);

it('接受64节点128关系8流程每流程32步的上限', () => {
  const data = architectureData();
  data.nodes = Array.from({ length: 64 }, (_, index) => ({
    ...data.nodes[0],
    id: `node${index}`,
  }));
  data.relations = Array.from({ length: 128 }, (_, index) => ({
    ...data.relations[0],
    from: 'node0',
    to: `node${index % 64}`,
    kind: index < 64 ? ('dependency' as const) : ('data_flow' as const),
  }));
  data.flows = Array.from({ length: 8 }, (_, index) => ({
    id: `flow${index}`,
    label: '设计路径',
    steps: Array.from({ length: 32 }, (_, order) => ({
      order: order + 1,
      node: 'node0',
      description: '重复查看此节点',
      source: architectureLocation(20),
    })),
  }));
  expect(validateArchitecture(data, architectureProject)).toBe(data);
});

it.each(['label', 'description', 'directory'] as const)(
  '%s按UTF8字节限制中文文本长度',
  (field) => {
    const data = architectureData();
    const limit = field === 'label' ? 256 : 1024;
    data.nodes[0][field] =
      '中'.repeat(Math.floor(limit / 3)) + 'a'.repeat(limit % 3);
    expect(validateArchitecture(data, architectureProject)).toBe(data);
    data.nodes[0][field] += '中';
    expect(() => validateArchitecture(data, architectureProject)).toThrow(
      '架构关系',
    );
  },
);

it('关系标签与步骤说明同样拒绝超限或空值', () => {
  const data = architectureData();
  data.relations[0].label = '中'.repeat(86);
  expect(() => validateArchitecture(data, architectureProject)).toThrow();
  data.relations[0].label = '依赖';
  data.flows[0].steps[0].description = '';
  expect(() => validateArchitecture(data, architectureProject)).toThrow();
  data.flows[0].steps[0].description = '中'.repeat(342);
  expect(() => validateArchitecture(data, architectureProject)).toThrow();
});
