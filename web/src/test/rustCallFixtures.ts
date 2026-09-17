import type {
  RustCallGraph,
  RustCallSitesData,
  RustSourceCoverage,
} from '../rustCallSites';

export function rustFile(path: string, bytes = 200): RustSourceCoverage {
  return {
    path,
    bytes,
    attributes: 0,
    conditional_attributes: 0,
    unexpanded_macros: 0,
    macro_definitions: 0,
    external_modules: 0,
  };
}

export function rustGraph(
  overrides: Partial<RustCallGraph> = {},
): RustCallGraph {
  const path = 'crates/core/internal/lib.rs';
  return {
    files: [
      {
        ...rustFile(path),
        attributes: 3,
        conditional_attributes: 2,
        unexpanded_macros: 1,
        macro_definitions: 1,
        external_modules: 1,
      },
      rustFile('src/empty.rs', 0),
    ],
    contexts: [
      {
        id: 'f0:root',
        parent: null,
        kind: 'file',
        name: null,
        source: { path, line: 1, column: 1 },
      },
      {
        id: 'f0:context:10:190',
        parent: 'f0:root',
        kind: 'function',
        name: '处理资料',
        source: { path, line: 2, column: 1 },
      },
      {
        id: 'f0:context:70:110',
        parent: 'f0:context:10:190',
        kind: 'closure',
        name: null,
        source: { path, line: 4, column: 10 },
      },
      {
        id: 'f1:root',
        parent: null,
        kind: 'file',
        name: null,
        source: { path: 'src/empty.rs', line: 1, column: 1 },
      },
    ],
    calls: [
      {
        id: 'f0:call:30:40',
        context: 'f0:context:10:190',
        kind: 'path',
        name: 'r#查询::读取',
        source: { path, line: 3, column: 13 },
      },
      {
        id: 'f0:call:50:65',
        context: 'f0:context:10:190',
        kind: 'method',
        name: '读取',
        source: { path, line: 3, column: 33 },
      },
      {
        id: 'f0:call:80:95',
        context: 'f0:context:70:110',
        kind: 'expression',
        name: null,
        source: { path, line: 4, column: 20 },
      },
      {
        id: 'f0:call:140:155',
        context: 'f0:context:10:190',
        kind: 'macro',
        name: '记录',
        source: { path, line: 6, column: 5 },
      },
    ],
    ...overrides,
  };
}

export function rustData(
  overrides: Partial<RustCallSitesData> = {},
): RustCallSitesData {
  return {
    status: 'ready',
    observed_at: '1789140200',
    graph: rustGraph(),
    diagnostics: [],
    ...overrides,
  };
}
