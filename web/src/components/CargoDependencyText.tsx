import { cargoKinds } from '../cargoDependencies';
import type { CargoDependencyGraph } from '../cargoDependencies';
import { CargoManifestSource } from './CargoManifestSource';

export function CargoDependencyText({
  graph,
}: {
  graph: CargoDependencyGraph;
}) {
  const packages = new Map(graph.packages.map((item) => [item.id, item.name]));
  return (
    <div
      className="cargo-text-data"
      role="region"
      aria-label="Cargo 清单依赖文本数据"
    >
      <h4>清单包</h4>
      <ul>
        {graph.packages.map((item) => (
          <li key={item.id}>
            <strong>{item.name}</strong> · <code>{item.id}</code>
            <CargoManifestSource source={item.source} />
          </li>
        ))}
      </ul>
      <h4>内部依赖声明</h4>
      <ul>
        {graph.dependencies.map((edge, index) => (
          <li key={index}>
            <strong>
              {packages.get(edge.from)} → {packages.get(edge.to)}
            </strong>
            <p>
              {cargoKinds[edge.kind]} · alias：{edge.alias} ·{' '}
              {edge.optional ? '可选依赖（optional）' : '非可选声明'}
            </p>
            <p>目标条件：{edge.target ?? '无目标条件'}</p>
            <CargoManifestSource source={edge.source} label="依赖引用来源" />
            {edge.inherited_from ? (
              <CargoManifestSource
                source={edge.inherited_from}
                label="继承定义来源"
              />
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
