import type { ArchitectureData } from '../architecture';

export const architectureProject = '项目/星舟';
export const architectureSource = {
  path: `${architectureProject}/01-框架/项目运作.md`,
  line: 1,
};
export const architectureLocation = (line: number) => ({
  ...architectureSource,
  line,
});

export function architectureData(
  overrides: Partial<ArchitectureData> = {},
): ArchitectureData {
  return {
    status: 'ready',
    source: architectureSource,
    nodes: [
      {
        id: 'web',
        label: '检索接口',
        kind: 'interface',
        directory: 'web',
        description: '接收查询并返回结果',
        source: architectureLocation(7),
      },
      {
        id: 'core',
        label: '资料索引',
        kind: 'module',
        directory: 'crates/core',
        description: '组织分类与资料',
        source: architectureLocation(8),
      },
      {
        id: 'export',
        label: '导出工具',
        kind: 'module',
        directory: 'scripts',
        description: '生成结果文件',
        source: architectureLocation(9),
      },
      {
        id: 'reader',
        label: '阅读者',
        kind: 'external',
        directory: null,
        description: '阅读并反馈',
        source: architectureLocation(10),
      },
      {
        id: 'detail',
        label: '深层资料',
        kind: 'data',
        directory: 'crates/core/internal',
        description: '未观测的子目录',
        source: architectureLocation(11),
      },
    ],
    relations: [
      {
        from: 'web',
        to: 'core',
        kind: 'dependency',
        label: '调用索引入口',
        source: architectureLocation(16),
      },
      {
        from: 'core',
        to: 'web',
        kind: 'data_flow',
        label: '返回查询资料',
        source: architectureLocation(17),
      },
      {
        from: 'core',
        to: 'core',
        kind: 'data_flow',
        label: '索引反馈自环',
        source: architectureLocation(18),
      },
      {
        from: 'core',
        to: 'export',
        kind: 'dependency',
        label: '组织导出结果',
        source: architectureLocation(19),
      },
    ],
    flows: [
      {
        id: 'search',
        label: '资料查询',
        steps: [
          {
            order: 1,
            node: 'web',
            description: '接收查询条件',
            source: architectureLocation(24),
          },
          {
            order: 2,
            node: 'core',
            description: '检索分类内容',
            source: architectureLocation(25),
          },
          {
            order: 3,
            node: 'web',
            description: '返回结果',
            source: architectureLocation(26),
          },
        ],
      },
      {
        id: 'export',
        label: '资料导出',
        steps: [
          {
            order: 1,
            node: 'core',
            description: '选择待导出分类',
            source: architectureLocation(27),
          },
          {
            order: 2,
            node: 'export',
            description: '生成分享资料',
            source: architectureLocation(28),
          },
        ],
      },
    ],
    diagnostics: [],
    ...overrides,
  };
}
