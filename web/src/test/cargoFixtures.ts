import type {
  CargoDependenciesData,
  CargoDependencyGraph,
} from '../cargoDependencies';

export function cargoGraph(
  overrides: Partial<CargoDependencyGraph> = {},
): CargoDependencyGraph {
  return {
    packages: [
      { id: '.', name: 'app', source: { path: 'Cargo.toml', line: 2 } },
      {
        id: 'crates/core',
        name: 'core',
        source: { path: 'crates/core/Cargo.toml', line: 3 },
      },
      {
        id: 'tools',
        name: 'tools',
        source: { path: 'tools/Cargo.toml', line: 4 },
      },
    ],
    dependencies: [
      {
        from: '.',
        to: 'crates/core',
        alias: 'renamed_core',
        kind: 'normal',
        target: null,
        optional: false,
        source: { path: 'Cargo.toml', line: 10 },
        inherited_from: { path: 'Cargo.toml', line: 40 },
      },
      {
        from: 'crates/core',
        to: '.',
        alias: 'app',
        kind: 'dev',
        target: 'cfg(target_os = "linux")',
        optional: false,
        source: { path: 'crates/core/Cargo.toml', line: 12 },
        inherited_from: null,
      },
      {
        from: '.',
        to: 'tools',
        alias: 'tools',
        kind: 'build',
        target: 'cfg(unix)',
        optional: true,
        source: { path: 'Cargo.toml', line: 20 },
        inherited_from: null,
      },
      {
        from: 'crates/core',
        to: 'crates/core',
        alias: 'core',
        kind: 'normal',
        target: null,
        optional: true,
        source: { path: 'crates/core/Cargo.toml', line: 24 },
        inherited_from: null,
      },
    ],
    external_dependencies: 3,
    ...overrides,
  };
}

export function cargoData(
  overrides: Partial<CargoDependenciesData> = {},
): CargoDependenciesData {
  return {
    status: 'ready',
    observed_at: '1789140000',
    graph: cargoGraph(),
    diagnostics: [],
    ...overrides,
  };
}
