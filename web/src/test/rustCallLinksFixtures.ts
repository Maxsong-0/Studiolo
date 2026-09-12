import type { RustCallAssociation, RustCallLinks } from '../rustCallLinks';
import type { RustCallGraph } from '../rustCallSites';
import { rustData, rustFile } from './rustCallFixtures';

export function rustLinkedGraph(): RustCallGraph {
  const path = 'src/lib.rs';
  return {
    files: [rustFile(path, 300), rustFile('src/other.rs', 0)],
    contexts: [
      {
        id: 'f0:root',
        kind: 'file',
        name: null,
        parent: null,
        source: { path, line: 1, column: 1 },
      },
      {
        id: 'f0:context:10:120',
        kind: 'function',
        name: 'main',
        parent: 'f0:root',
        source: { path, line: 2, column: 1 },
      },
      {
        id: 'f0:context:130:200',
        kind: 'function',
        name: '读取',
        parent: 'f0:root',
        source: { path, line: 8, column: 1 },
      },
      {
        id: 'f0:context:210:250',
        kind: 'function',
        name: '无引用',
        parent: 'f0:root',
        source: { path, line: 12, column: 1 },
      },
      {
        id: 'f1:root',
        kind: 'file',
        name: null,
        parent: null,
        source: { path: 'src/other.rs', line: 1, column: 1 },
      },
    ],
    calls: [
      {
        id: 'f0:call:30:40',
        kind: 'path',
        name: '读取',
        context: 'f0:context:10:120',
        source: { path, line: 3, column: 5 },
      },
      {
        id: 'f0:call:50:60',
        kind: 'path',
        name: 'r#读取',
        context: 'f0:context:10:120',
        source: { path, line: 4, column: 5 },
      },
      {
        id: 'f0:call:80:90',
        kind: 'path',
        name: 'missing',
        context: 'f0:context:10:120',
        source: { path, line: 5, column: 5 },
      },
      {
        id: 'f0:call:100:110',
        kind: 'method',
        name: 'fetch',
        context: 'f0:context:10:120',
        source: { path, line: 6, column: 5 },
      },
      {
        id: 'f0:call:150:160',
        kind: 'path',
        name: '读取',
        context: 'f0:context:130:200',
        source: { path, line: 9, column: 5 },
      },
      {
        id: 'f0:call:180:190',
        kind: 'expression',
        name: null,
        context: 'f0:context:130:200',
        source: { path, line: 10, column: 5 },
      },
    ],
  };
}

export function rustLinks(
  overrides: Partial<RustCallLinks> = {},
): RustCallLinks {
  return {
    version: 1,
    batch_id: 'a'.repeat(64),
    associations: rustLinkedGraph().calls.map<RustCallAssociation>(
      (call, index) =>
        [0, 1, 4].includes(index)
          ? {
              call_id: call.id,
              status: 'linked',
              target: 'f0:context:130:200',
              reason: null,
            }
          : {
              call_id: call.id,
              status: 'unresolved',
              target: null,
              reason: index === 2 ? 'not_found' : 'callee_form',
            },
    ),
    ...overrides,
  };
}

export const rustAnalysis = () =>
  rustData({ graph: rustLinkedGraph(), links: rustLinks() });
