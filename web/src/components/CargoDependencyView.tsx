import { useState } from 'react';
import { cargoKinds } from '../cargoDependencies';
import type {
  CargoDependency,
  CargoDependencyGraph,
} from '../cargoDependencies';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { CargoManifestSource } from './CargoManifestSource';
import { CargoPackageDetails } from './CargoPackageDetails';
import { Icon } from './Icon';

export function CargoDependencyView({
  graph,
  data,
  onOpen,
}: {
  graph: CargoDependencyGraph;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const [kind, setKind] = useState<'all' | CargoDependency['kind']>('all');
  const packages = new Map(graph.packages.map((item) => [item.id, item]));
  const selected = packages.get(selection) ?? graph.packages[0];
  const edges = graph.dependencies.filter(
    (item) => kind === 'all' || item.kind === kind,
  );
  return (
    <div className="cargo-dependency-layout">
      <div className="cargo-dependency-main">
        <div className="cargo-packages" role="group" aria-label="Cargo 包">
          {graph.packages.map((item) => (
            <button
              key={item.id}
              className="cargo-package"
              aria-pressed={selected.id === item.id}
              aria-label={`选择 Cargo 包 ${item.name}`}
              onClick={() => setSelection(item.id)}
            >
              <strong>{item.name}</strong>
              <code>{item.id}</code>
            </button>
          ))}
        </div>
        <div className="cargo-filter">
          <label htmlFor="cargo-dependency-kind">清单依赖种类</label>
          <select
            id="cargo-dependency-kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as typeof kind)}
          >
            <option value="all">全部种类</option>
            <option value="normal">普通依赖（normal）</option>
            <option value="dev">开发依赖（dev）</option>
            <option value="build">构建依赖（build）</option>
          </select>
          <span className="operation-muted">{edges.length} 条内部依赖声明</span>
        </div>
        {edges.length ? (
          <ul className="cargo-edge-list" aria-label="Cargo 内部依赖声明">
            {edges.map((edge, index) => (
              <li key={index}>
                <div className="cargo-edge-direction">
                  <button
                    onClick={() => setSelection(edge.from)}
                    aria-label={`选择依赖起点包 ${packages.get(edge.from)!.name}`}
                  >
                    {packages.get(edge.from)!.name}
                  </button>
                  <Icon name="arrow" />
                  <button
                    onClick={() => setSelection(edge.to)}
                    aria-label={`选择依赖终点包 ${packages.get(edge.to)!.name}`}
                  >
                    {packages.get(edge.to)!.name}
                  </button>
                </div>
                <p>
                  {cargoKinds[edge.kind]} · alias：<code>{edge.alias}</code> ·{' '}
                  {edge.optional ? '可选依赖（optional）' : '非可选声明'}
                </p>
                <p>
                  目标条件：
                  <span className="cargo-target">
                    {edge.target ?? '无目标条件'}
                  </span>
                </p>
                <CargoManifestSource
                  source={edge.source}
                  label="依赖引用来源"
                />
                {edge.inherited_from ? (
                  <CargoManifestSource
                    source={edge.inherited_from}
                    label="继承定义来源"
                  />
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="operation-muted" role="status">
            当前种类没有内部依赖声明。
          </p>
        )}
      </div>
      <CargoPackageDetails item={selected} data={data} onOpen={onOpen} />
    </div>
  );
}
