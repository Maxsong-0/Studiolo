import type { CargoPackage } from '../cargoDependencies';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { CargoManifestSource } from './CargoManifestSource';

export function CargoPackageDetails({
  item,
  data,
  onOpen,
}: {
  item: CargoPackage;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const nodes =
    data.architecture?.status === 'ready'
      ? data.architecture.nodes.filter((node) => node.directory === item.id)
      : [];
  return (
    <aside
      className="cargo-package-detail"
      aria-label="所选 Cargo 包"
      aria-live="polite"
    >
      <span className="operation-eyebrow">Cargo 包 · 清单派生</span>
      <h3>{item.name}</h3>
      <p>
        包目录：<code>{item.id}</code>
      </p>
      <CargoManifestSource source={item.source} />
      <h4>精确关联的架构声明</h4>
      {nodes.length ? (
        <ul aria-label="关联的架构节点">
          {nodes.map((node) => (
            <li key={node.id}>
              <strong>{node.label}</strong>
              <p>{node.description}</p>
              <button
                className="text-button"
                onClick={() => onOpen(node.source)}
                aria-label={`${node.label}，查看关联的 Vault 声明`}
              >
                查看 Vault 声明原文
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="operation-muted">
          没有目录与此包 ID 精确匹配的架构节点。
        </p>
      )}
      <p className="operation-muted">
        目录关联不代表设计依赖与实现一致，也不扩大目录核验范围。
      </p>
    </aside>
  );
}
