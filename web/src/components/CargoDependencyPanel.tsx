import type { CargoDependenciesData } from '../cargoDependencies';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { CellHeader } from '../ui';
import { CargoDependencyBoundary } from './CargoDependencyBoundary';
import { CargoDependencyView } from './CargoDependencyView';
import '../styles/operations.css';
import '../styles/cargo-dependencies.css';

const unavailable: Record<
  Exclude<CargoDependenciesData['status'], 'ready'>,
  [string, string]
> = {
  disabled: [
    '尚未启用 Cargo 清单读取',
    '此仓库绑定未启用清单依赖采集，目前没有清单观测结果。',
  ],
  unbound: ['仓库未绑定', '此项目尚无匹配的仓库绑定，无法读取 Cargo 清单。'],
  unavailable: [
    'Cargo 清单暂不可用',
    '清单缺失、不可读或读取期间发生变化，请核对仓库后重新核验。',
  ],
  invalid: [
    'Cargo 清单依赖声明无效',
    '本次清单解析或依赖引用校验未通过，未发布部分依赖图。',
  ],
  unsupported: [
    '清单包含暂不支持的语义',
    '本次清单超出受支持的显式成员范围或依赖语义，未发布部分依赖图。',
  ],
};

/**
 * 读不到（含读取预算拒绝）、无效或超出支持范围（含派生预算超限）是「出问题」：标题前加红色方块，
 * 这是本块唯一用红的地方；未启用与未绑定只是没有开这项能力，保持中性。
 */
const problems: ReadonlySet<CargoDependenciesData['status']> = new Set([
  'unavailable',
  'invalid',
  'unsupported',
]);

/**
 * Cargo 清单依赖：运作页证据格里的一块（格子由运作页给出，这里不再套框）。
 * 标题行用 CellHeader；只有 ready 才显示观测时间与图，其余状态各自降级并说明原因。
 */
export function CargoDependencyPanel({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const cargo = data.cargo_dependencies;
  const graph = cargo?.status === 'ready' ? cargo.graph : null;
  const reason = !cargo
    ? ['暂无 Cargo 清单依赖数据', '当前服务未提供 Cargo 清单依赖观测。']
    : cargo.status === 'ready'
      ? null
      : unavailable[cargo.status];
  const problem = cargo !== undefined && problems.has(cargo.status);
  return (
    <section
      className="operation-panel cargo-panel"
      aria-labelledby="cargo-dependencies-title"
    >
      <CellHeader
        title={<span id="cargo-dependencies-title">Cargo 清单依赖</span>}
        meta="仓库清单派生"
      />
      <p className="operation-muted">
        从 Cargo
        清单中的本地包声明派生，保留目标条件和可选声明；不代表源码调用或实际构建图。
      </p>
      {graph &&
      cargo?.observed_at !== null &&
      cargo?.observed_at !== undefined ? (
        <>
          <div className="cargo-readout">
            <p className="cargo-observation">
              清单观测：
              <time
                dateTime={new Date(
                  Number(cargo.observed_at) * 1000,
                ).toISOString()}
              >
                {new Date(Number(cargo.observed_at) * 1000).toLocaleString()}
              </time>
              ；文档声明使用快照 {data.generation}。
            </p>
            <p className="cargo-external-count">
              外部依赖声明：{graph.external_dependencies}{' '}
              条。按别名、种类和目标条件计数，不是唯一包数量。
            </p>
          </div>
          <p className="operation-muted">
            清单来源显示仓库相对路径与行号；关联的 Vault 声明可单独打开。
          </p>
          <CargoDependencyBoundary
            key={`${data.project}:${data.generation}:${cargo.observed_at}`}
            graph={graph}
          >
            <CargoDependencyView graph={graph} data={data} onOpen={onOpen} />
          </CargoDependencyBoundary>
        </>
      ) : (
        <div
          className={
            problem
              ? 'operation-unavailable is-problem'
              : 'operation-unavailable'
          }
          role="status"
        >
          <h3>{reason?.[0]}</h3>
          <p>{reason?.[1]}</p>
        </div>
      )}
      {cargo?.diagnostics.length ? (
        <details className="operation-diagnostics" open>
          <summary>清单观测说明（{cargo.diagnostics.length}）</summary>
          <ul>
            {cargo.diagnostics.map((message, index) => (
              <li key={index}>{message}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}
