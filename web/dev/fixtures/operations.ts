/**
 * `/api/project-operations`：项目运作（结构声明、目录漂移、架构、Cargo 依赖、Rust 调用位置）。
 *
 * 只有 Studiolo 绑定了代码仓库：目录声明来自 `01-框架/目录结构.md` 的 text 目录树，架构来自
 * `01-框架/项目运作.md` 的三张表；Cargo 依赖图按本仓库 23 个成员的真实清单生成（内部依赖 74 条、
 * 外部依赖 193 条，行号即清单行号）；Rust 调用位置取 `crates/local-server/src/health.rs` 的真实
 * 字节区间。其余项目未绑定仓库、没有声明，按后端原话给出「不可用 / 未绑定」。
 */
import { sha256 } from './ids.ts';
import type { VaultNote } from './vault.ts';

const lineWith = (note: VaultNote, needle: string) => {
  const index = note.content
    .split('\n')
    .findIndex((line) => line.includes(needle));
  if (index < 0) throw new Error(`假数据笔记缺少「${needle}」`);
  return index + 1;
};

/** [`crates/` 下的成员目录, 包名]；每份清单的 `name =` 都在第 2 行。 */
const PACKAGES: [string, string][] = [
  ['core', 'studiolo-core'],
  ['process-safety', 'studiolo-process-safety'],
  ['vault-broker', 'studiolo-vault-broker'],
  ['vault-admin', 'studiolo-vault-admin'],
  ['vault-index', 'studiolo-vault-index'],
  ['control-db', 'studiolo-control-db'],
  ['audit-ledger', 'studiolo-audit-ledger'],
  ['auth', 'studiolo-auth'],
  ['auth-admin', 'studiolo-auth-admin'],
  ['host-config', 'studiolo-host-config'],
  ['sync-bridge', 'studiolo-sync-bridge'],
  ['sync-transport', 'studiolo-sync-transport'],
  ['backup', 'studiolo-backup'],
  ['observability', 'studiolo-observability'],
  ['policy', 'studiolo-policy'],
  ['runner-adapter', 'studiolo-runner-adapter'],
  ['gateway', 'studiolo-gateway'],
  ['task-engine', 'studiolo-task-engine'],
  ['studiolo', 'studiolo'],
  ['workbench', 'studiolo-workbench'],
  ['local-server', 'studiolo-local-server'],
  ['supply', 'studiolo-supply'],
  ['agent-server', 'studiolo-agent-server'],
];

/** [起点, 终点, 种类, 清单行号, 工作区声明行号]；别名即终点包名。 */
const DEPENDENCIES: [
  string,
  string,
  'normal' | 'dev',
  number,
  number | null,
][] = [
  ['vault-broker', 'core', 'normal', 17, 41],
  ['vault-admin', 'core', 'normal', 11, 41],
  ['vault-admin', 'vault-broker', 'normal', 12, 44],
  ['vault-admin', 'host-config', 'normal', 13, 49],
  ['vault-admin', 'audit-ledger', 'normal', 14, 47],
  ['vault-admin', 'process-safety', 'normal', 15, 43],
  ['vault-index', 'core', 'normal', 16, 41],
  ['control-db', 'core', 'normal', 16, 41],
  ['audit-ledger', 'core', 'normal', 16, 41],
  ['auth', 'audit-ledger', 'normal', 22, 47],
  ['auth-admin', 'auth', 'normal', 13, 48],
  ['auth-admin', 'host-config', 'normal', 14, 49],
  ['auth-admin', 'process-safety', 'normal', 15, 43],
  ['auth-admin', 'audit-ledger', 'normal', 16, 47],
  ['host-config', 'core', 'normal', 10, 41],
  ['host-config', 'auth', 'normal', 11, 48],
  ['sync-bridge', 'core', 'normal', 15, 41],
  ['sync-bridge', 'audit-ledger', 'normal', 16, 47],
  ['sync-transport', 'core', 'normal', 15, 41],
  ['sync-transport', 'host-config', 'normal', 16, 49],
  ['backup', 'core', 'normal', 15, 41],
  ['backup', 'audit-ledger', 'normal', 16, 47],
  ['observability', 'core', 'normal', 15, 41],
  ['observability', 'audit-ledger', 'normal', 16, 47],
  ['policy', 'core', 'normal', 15, 41],
  ['policy', 'audit-ledger', 'normal', 16, 47],
  ['runner-adapter', 'core', 'normal', 15, 41],
  ['gateway', 'core', 'normal', 15, 41],
  ['gateway', 'audit-ledger', 'normal', 16, 47],
  ['gateway', 'process-safety', 'normal', 17, 43],
  ['task-engine', 'core', 'normal', 16, 41],
  ['task-engine', 'control-db', 'normal', 17, 46],
  ['task-engine', 'vault-broker', 'normal', 18, 44],
  ['task-engine', 'audit-ledger', 'normal', 19, 47],
  ['task-engine', 'runner-adapter', 'normal', 20, 55],
  ['studiolo', 'core', 'normal', 17, 41],
  ['studiolo', 'vault-broker', 'normal', 18, 44],
  ['studiolo', 'vault-index', 'normal', 19, 45],
  ['studiolo', 'control-db', 'normal', 20, 46],
  ['studiolo', 'audit-ledger', 'normal', 21, 47],
  ['studiolo', 'runner-adapter', 'normal', 22, 55],
  ['studiolo', 'gateway', 'normal', 24, 56],
  ['studiolo', 'task-engine', 'normal', 25, 57],
  ['workbench', 'core', 'normal', 25, 41],
  ['workbench', 'vault-index', 'normal', 26, 45],
  ['local-server', 'control-db', 'normal', 16, 46],
  ['local-server', 'runner-adapter', 'normal', 17, 55],
  ['local-server', 'gateway', 'normal', 18, 56],
  ['local-server', 'task-engine', 'normal', 19, 57],
  ['local-server', 'supply', 'normal', 20, 59],
  ['local-server', 'auth', 'normal', 34, 48],
  ['local-server', 'host-config', 'normal', 35, 49],
  ['local-server', 'process-safety', 'normal', 36, 43],
  ['local-server', 'audit-ledger', 'normal', 37, 47],
  ['local-server', 'core', 'normal', 38, 41],
  ['local-server', 'vault-broker', 'normal', 39, 44],
  ['local-server', 'vault-index', 'normal', 40, 45],
  ['local-server', 'workbench', 'normal', 41, 58],
  ['local-server', 'backup', 'normal', 42, 52],
  ['local-server', 'observability', 'normal', 43, 53],
  ['local-server', 'sync-transport', 'normal', 44, 51],
  ['local-server', 'policy', 'normal', 45, 54],
  ['local-server', 'studiolo', 'dev', 50, null],
  ['local-server', 'auth', 'dev', 54, 48],
  ['supply', 'core', 'normal', 15, 41],
  ['supply', 'vault-broker', 'normal', 16, 44],
  ['supply', 'vault-index', 'normal', 17, 45],
  ['supply', 'workbench', 'normal', 18, 58],
  ['supply', 'audit-ledger', 'normal', 19, 47],
  ['agent-server', 'supply', 'normal', 23, 59],
  ['agent-server', 'policy', 'normal', 24, 54],
  ['agent-server', 'process-safety', 'normal', 27, 43],
  ['agent-server', 'audit-ledger', 'normal', 28, 47],
  ['agent-server', 'core', 'normal', 29, 41],
];

const member = (name: string) => `crates/${name}`;
const packageName = new Map(PACKAGES);

function cargoGraph() {
  return {
    packages: PACKAGES.map(([id, name]) => ({
      id: member(id),
      name,
      source: { path: `${member(id)}/Cargo.toml`, line: 2 },
    })),
    dependencies: DEPENDENCIES.map(([from, to, kind, line, inherited]) => ({
      from: member(from),
      to: member(to),
      alias: packageName.get(to)!,
      kind,
      target: null,
      optional: false,
      source: { path: `${member(from)}/Cargo.toml`, line },
      inherited_from:
        inherited === null ? null : { path: 'Cargo.toml', line: inherited },
    })),
    external_dependencies: 193,
  };
}

const RUST_FILE = 'crates/local-server/src/health.rs';
const PAGE = 'f0:context:3176:3949';
const ROUTES = 'f0:context:2310:2638';
const EVIDENCE = 'f0:context:2906:3052';
const at = (line: number, column: number) => ({
  path: RUST_FILE,
  line,
  column,
});

/** [ID, 上下文, 种类, 名称, 行, 列, 词法关联（函数上下文 ID 或未解析原因）]。 */
const CALLS: [string, string, string, string, number, number, string][] = [
  [
    'f0:call:2531:2562',
    ROUTES,
    'path',
    'enabled',
    57,
    8,
    'f0:context:1803:1918',
  ],
  ['f0:call:2573:2602', ROUTES, 'method', 'route', 58, 9, 'callee_form'],
  ['f0:call:2592:2601', ROUTES, 'path', 'get', 58, 28, 'shadowed_name'],
  ['f0:call:2998:3050', EVIDENCE, 'method', 'layer', 69, 5, 'callee_form'],
  ['f0:call:3008:3049', EVIDENCE, 'path', 'Extension', 69, 15, 'shadowed_name'],
  [
    'f0:call:3018:3048',
    EVIDENCE,
    'path',
    'HealthEvidenceSource',
    69,
    25,
    'shadowed_name',
  ],
  ['f0:call:3455:3635', PAGE, 'method', 'into_response', 81, 16, 'callee_form'],
  ['f0:call:3504:3595', PAGE, 'path', 'Json', 83, 13, 'shadowed_name'],
  [
    'f0:call:3749:3803',
    PAGE,
    'path',
    'tokio::task::spawn_blocking',
    90,
    20,
    'callee_form',
  ],
  [
    'f0:call:3749:3876',
    PAGE,
    'method',
    'unwrap_or_else',
    90,
    20,
    'callee_form',
  ],
  [
    'f0:call:3785:3802',
    'f0:context:3777:3802',
    'method',
    'snapshot',
    90,
    56,
    'callee_form',
  ],
  [
    'f0:call:3847:3875',
    'f0:context:3843:3875',
    'path',
    'EvidenceSnapshot::unknown',
    92,
    29,
    'callee_form',
  ],
  ['f0:call:3882:3931', PAGE, 'path', 'Json', 93, 5, 'shadowed_name'],
  ['f0:call:3882:3947', PAGE, 'method', 'into_response', 93, 5, 'callee_form'],
  [
    'f0:call:3887:3930',
    PAGE,
    'path',
    'HealthBody::from_report',
    93,
    10,
    'callee_form',
  ],
  ['f0:call:3912:3929', PAGE, 'method', 'report', 93, 35, 'callee_form'],
];

function rustCallSites(observedAt: string) {
  return {
    status: 'ready',
    observed_at: observedAt,
    graph: {
      files: [
        {
          path: RUST_FILE,
          bytes: 3975,
          attributes: 4,
          conditional_attributes: 1,
          unexpanded_macros: 0,
          macro_definitions: 0,
          external_modules: 3,
        },
      ],
      contexts: [
        {
          id: 'f0:root',
          parent: null,
          kind: 'file',
          name: null,
          source: at(1, 1),
        },
        {
          id: 'f0:context:1803:1918',
          parent: 'f0:root',
          kind: 'function',
          name: 'enabled',
          source: at(43, 1),
        },
        {
          id: ROUTES,
          parent: 'f0:root',
          kind: 'function',
          name: 'routes',
          source: at(52, 1),
        },
        {
          id: EVIDENCE,
          parent: 'f0:root',
          kind: 'function',
          name: 'with_health_evidence',
          source: at(68, 1),
        },
        {
          id: PAGE,
          parent: 'f0:root',
          kind: 'function',
          name: 'page',
          source: at(78, 1),
        },
        {
          id: 'f0:context:3777:3802',
          parent: PAGE,
          kind: 'closure',
          name: null,
          source: at(90, 48),
        },
        {
          id: 'f0:context:3843:3875',
          parent: PAGE,
          kind: 'closure',
          name: null,
          source: at(92, 25),
        },
      ],
      calls: CALLS.map(([id, context, kind, name, line, column]) => ({
        id,
        context,
        kind,
        name,
        source: at(line, column),
      })),
    },
    links: {
      version: 1,
      batch_id: sha256('rust links health.rs'),
      associations: CALLS.map(([id, , , , , , link]) =>
        link.startsWith('f0:')
          ? { call_id: id, status: 'linked', target: link, reason: null }
          : { call_id: id, status: 'unresolved', target: null, reason: link },
      ),
    },
    diagnostics: [],
  };
}

const DECLARED = [
  'crates',
  'crates/local-server',
  'crates/vault-broker',
  'crates/task-engine',
  'crates/policy',
  'crates/supply',
  'crates/agent-server',
  'web',
  'deploy',
  'scripts',
];
/** 仓库里真实存在但声明没写到的目录（按比较父目录 `.` 与 `crates` 观测）。 */
const UNDOCUMENTED = [
  'docs',
  'droppy-code-main',
  'examples',
  ...[
    'core',
    'process-safety',
    'vault-admin',
    'vault-index',
    'control-db',
    'audit-ledger',
    'auth',
    'auth-admin',
    'host-config',
    'sync-bridge',
    'sync-transport',
    'backup',
    'observability',
    'runner-adapter',
    'gateway',
    'studiolo',
    'workbench',
  ].map(member),
];

export function projectOperations(
  project: string,
  generation: number,
  notes: VaultNote[],
  now: number,
) {
  const tree = notes.find(
    (note) => note.path === `${project}/01-框架/目录结构.md`,
  );
  const design = notes.find(
    (note) => note.path === `${project}/01-框架/项目运作.md`,
  );
  const observedAt = String(Math.floor(now / 1000));
  if (!tree || !design)
    return {
      generation,
      project,
      declaration_status: 'unavailable',
      declaration_source: null,
      directories: [],
      comparison_parents: [],
      diagnostics: ['缺少 01-框架/目录结构.md，目录声明不可用'],
      repository: { status: 'unbound', observed_at: null },
      drift: [],
      architecture: {
        status: 'unavailable',
        source: null,
        nodes: [],
        relations: [],
        flows: [],
        diagnostics: ['缺少可选的 01-框架/项目运作.md，设计声明不可用'],
      },
      cargo_dependencies: {
        status: 'unbound',
        observed_at: null,
        graph: null,
        diagnostics: ['当前项目未绑定仓库，Cargo 清单证据不可用。'],
      },
      rust_call_sites: {
        status: 'unbound',
        observed_at: null,
        graph: null,
        links: null,
        diagnostics: ['当前项目未绑定仓库，Rust 调用位置证据不可用。'],
      },
    };
  const treeSource = (line: number) => ({ path: tree.path, line });
  const directories = DECLARED.map((path) => {
    const name = path.split('/').at(-1)!;
    const line = lineWith(tree, `── ${name}/ #`);
    const label = tree.content.split('\n')[line - 1].split(' # ')[1];
    return { path, label, source: treeSource(line) };
  });
  const sourceOf = new Map(directories.map((item) => [item.path, item.source]));
  const drift = [
    ...DECLARED.map((path) => ({
      path,
      status: 'present',
      source: sourceOf.get(path)!,
    })),
    ...UNDOCUMENTED.map((path) => ({
      path,
      status: 'undocumented',
      source: null,
    })),
  ].sort((left, right) => (left.path < right.path ? -1 : 1));
  const designSource = (needle: string) => ({
    path: design.path,
    line: lineWith(design, needle),
  });
  const rows = (header: string) => {
    const lines = design.content.split('\n');
    const start = lines.findIndex((line) => line.startsWith(header)) + 2;
    const result: { cells: string[]; line: number }[] = [];
    for (let index = start; lines[index]?.startsWith('| '); index += 1)
      result.push({
        cells: lines[index]
          .slice(1, -1)
          .split('|')
          .map((cell) => cell.trim()),
        line: index + 1,
      });
    return result;
  };
  const kinds: Record<string, string> = {
    模块: 'module',
    接口: 'interface',
    外部: 'external',
    数据: 'data',
  };
  const flows = new Map<
    string,
    { id: string; label: string; steps: unknown[] }
  >();
  for (const { cells, line } of rows('| 流程ID |')) {
    const flow = flows.get(cells[0]) ?? {
      id: cells[0],
      label: cells[1],
      steps: [],
    };
    flow.steps.push({
      order: Number(cells[2]),
      node: cells[3],
      description: cells[4],
      source: { path: design.path, line },
    });
    flows.set(cells[0], flow);
  }
  return {
    generation,
    project,
    declaration_status: 'ready',
    declaration_source: treeSource(lineWith(tree, '```text')),
    directories,
    comparison_parents: ['.', 'crates'],
    diagnostics: [],
    repository: { status: 'ready', observed_at: observedAt },
    drift,
    architecture: {
      status: 'ready',
      source: designSource('# Studiolo 项目运作'),
      nodes: rows('| ID |').map(({ cells, line }) => ({
        id: cells[0],
        label: cells[1],
        kind: kinds[cells[2]],
        directory: cells[3] === '-' ? null : cells[3],
        description: cells[4],
        source: { path: design.path, line },
      })),
      relations: rows('| 起点 |').map(({ cells, line }) => ({
        from: cells[0],
        to: cells[1],
        kind: cells[2] === '依赖' ? 'dependency' : 'data_flow',
        label: cells[3],
        source: { path: design.path, line },
      })),
      flows: [...flows.values()],
      diagnostics: [],
    },
    cargo_dependencies: {
      status: 'ready',
      observed_at: observedAt,
      graph: cargoGraph(),
      diagnostics: [],
    },
    rust_call_sites: rustCallSites(observedAt),
  };
}
