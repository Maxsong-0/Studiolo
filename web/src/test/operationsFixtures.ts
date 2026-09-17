import type { ProjectOperationsData } from '../projectOperations';
import { fixture } from './fixtures';

export const declarationSource = {
  path: '项目/星舟/01-设计/架构.md',
  line: 12,
};
export const directorySource = { ...declarationSource, line: 16 };

export function operationsData(
  overrides: Partial<ProjectOperationsData> = {},
): ProjectOperationsData {
  return {
    generation: 7,
    project: fixture.projects[0].root,
    declaration_status: 'ready',
    declaration_source: declarationSource,
    directories: [
      { path: 'crates', label: 'crates', source: null },
      {
        path: 'crates/core',
        label: '管理项目结构与文档快照',
        source: directorySource,
      },
      {
        path: 'web',
        label: '呈现只读工作台',
        source: { ...directorySource, line: 17 },
      },
    ],
    comparison_parents: ['.', 'crates'],
    diagnostics: [],
    repository: { status: 'ready', observed_at: '1789140000' },
    drift: [
      { path: 'crates/core', status: 'present', source: directorySource },
      {
        path: 'web',
        status: 'missing',
        source: { ...directorySource, line: 17 },
      },
      { path: 'scripts', status: 'undocumented', source: null },
    ],
    ...overrides,
  };
}
