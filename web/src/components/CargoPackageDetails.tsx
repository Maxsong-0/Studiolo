import type { CargoPackage } from '../cargoDependencies';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Surface } from '../ui';
import { CargoManifestSource } from './CargoManifestSource';
import '../styles/operations.css';
import '../styles/cargo-dependencies.css';

/**
 * 所选包的侧栏（Surface 框）：包目录与清单来源是仓库文本；只有按包 ID 与架构节点目录精确相等的声明
 * 才列为关联，并各自给出 Vault 原文入口。侧栏常驻在页面里，不做出现动效；换包时读屏礼貌播报。
 */
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
    <Surface
      as="aside"
      appear={false}
      className="cargo-package-detail"
      aria-label="所选 Cargo 包"
      aria-live="polite"
    >
      <span className="cargo-detail-eyebrow">Cargo 包 · 清单派生</span>
      <h3>{item.name}</h3>
      <div className="cargo-detail-facts">
        <p className="cargo-detail-fact">
          <span className="cargo-detail-label">包目录：</span>
          <code>{item.id}</code>
        </p>
        <CargoManifestSource source={item.source} />
      </div>
      <h4>精确关联的架构声明</h4>
      {nodes.length ? (
        <ul className="cargo-detail-nodes" aria-label="关联的架构节点">
          {nodes.map((node) => (
            <li key={node.id}>
              <strong>{node.label}</strong>
              <p>{node.description}</p>
              <Button
                variant="text"
                size="sm"
                onClick={() => onOpen(node.source)}
                aria-label={`${node.label}，查看关联的 Vault 声明`}
              >
                查看 Vault 声明原文
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="operation-muted">
          没有目录与此包 ID 精确匹配的架构节点。
        </p>
      )}
      <p className="operation-muted cargo-detail-note">
        目录关联不代表设计依赖与实现一致，也不扩大目录核验范围。
      </p>
    </Surface>
  );
}
