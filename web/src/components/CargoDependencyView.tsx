import { useState } from 'react';
import { cargoKinds } from '../cargoDependencies';
import type {
  CargoDependency,
  CargoDependencyGraph,
} from '../cargoDependencies';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button } from '../ui';
import { CargoManifestSource } from './CargoManifestSource';
import { CargoPackageDetails } from './CargoPackageDetails';
import '../styles/projects.css';
import '../styles/operations.css';
import '../styles/cargo-dependencies.css';

/**
 * 两张表默认只铺前这么多行：真实仓库几十个包、上百条依赖一次铺开，运作页会被拉到上万像素。
 * 折起来的行仍在接口数据里，点「显示全部」原样展开；筛选、选择与来源一条不少。
 */
const FOLD = 12;

/**
 * 包表（选包）与所选包侧栏并排，内部依赖表在下面占满整行。
 * 两张表保留原来的语义结构（包是按钮组、依赖是列表）；表头只给眼睛看，读屏靠每格自带的标签，
 * 窄屏时这些标签直接显示出来。DOM 顺序仍是包、筛选、依赖、侧栏，键盘顺序与原来一致。
 */
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
  const [allPackages, setAllPackages] = useState(false);
  const [allEdges, setAllEdges] = useState(false);
  const packages = new Map(graph.packages.map((item) => [item.id, item]));
  const selected = packages.get(selection) ?? graph.packages[0];
  const edges = graph.dependencies.filter(
    (item) => kind === 'all' || item.kind === kind,
  );
  const foldPackages = !allPackages && graph.packages.length > FOLD;
  // 折起时所选包仍要看得见：它排在前几行之外，就接在后面单独显示。
  const packageRows = foldPackages
    ? graph.packages.indexOf(selected) >= FOLD
      ? [...graph.packages.slice(0, FOLD), selected]
      : graph.packages.slice(0, FOLD)
    : graph.packages;
  const edgeRows =
    !allEdges && edges.length > FOLD ? edges.slice(0, FOLD) : edges;
  const endpoint = (id: string, role: '起点' | '终点') => (
    <button
      type="button"
      className="cargo-endpoint"
      data-selected={id === selected.id ? 'true' : undefined}
      onClick={() => setSelection(id)}
      aria-label={`选择依赖${role}包 ${packages.get(id)!.name}`}
    >
      {packages.get(id)!.name}
    </button>
  );
  return (
    <div className="cargo-dependency-layout">
      <div className="cargo-packages-area">
        <div
          id="cargo-packages"
          className="cargo-table cargo-package-table"
          role="group"
          aria-label="Cargo 包"
        >
          <div className="cargo-table__head" aria-hidden="true">
            <span>包 · 包目录</span>
            <span>仓库清单来源</span>
          </div>
          {packageRows.map((item) => {
            const pressed = selected.id === item.id;
            return (
              <div
                key={item.id}
                className="cargo-package-row"
                data-selected={pressed ? 'true' : undefined}
              >
                <button
                  type="button"
                  className="cargo-package"
                  aria-pressed={pressed}
                  aria-label={`选择 Cargo 包 ${item.name}`}
                  onClick={() => setSelection(item.id)}
                >
                  <strong>{item.name}</strong>
                  <code>{item.id}</code>
                </button>
                <CargoManifestSource source={item.source} />
              </div>
            );
          })}
        </div>
        {graph.packages.length > FOLD ? (
          <div className="cargo-fold">
            <Button
              size="sm"
              aria-expanded={allPackages}
              aria-controls="cargo-packages"
              onClick={() => setAllPackages((value) => !value)}
            >
              {allPackages
                ? `收起，只显示前 ${FOLD} 个包`
                : `显示全部 ${graph.packages.length} 个包`}
            </Button>
            <span className="cargo-fold__count">
              显示 {packageRows.length} / {graph.packages.length} 个包
            </span>
          </div>
        ) : null}
      </div>
      <div className="cargo-edges-area">
        <div className="cargo-filter">
          <label htmlFor="cargo-dependency-kind">清单依赖种类</label>
          <span className="project-field project-field--select cargo-select">
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
          </span>
          <span className="cargo-filter__count">
            {edges.length} 条内部依赖声明
          </span>
        </div>
        {edges.length ? (
          <div className="cargo-table cargo-edge-table">
            <div className="cargo-table__head" aria-hidden="true">
              <span>方向（起点 → 终点）</span>
              <span>种类</span>
              <span>别名 · 可选</span>
              <span>目标条件</span>
              <span>仓库清单来源</span>
            </div>
            <ul
              id="cargo-edges"
              className="cargo-edge-list"
              aria-label="Cargo 内部依赖声明"
            >
              {edgeRows.map((edge, index) => (
                <li key={index}>
                  <div className="cargo-edge-direction">
                    {endpoint(edge.from, '起点')}
                    <span className="cargo-edge-arrow" aria-hidden="true" />
                    {endpoint(edge.to, '终点')}
                  </div>
                  <p className="cargo-edge-kind">
                    <span className="cargo-cell-label">种类：</span>
                    {cargoKinds[edge.kind]}
                  </p>
                  <p className="cargo-edge-alias">
                    <span className="cargo-cell-label">alias：</span>
                    <code>{edge.alias}</code>
                    <span
                      className={
                        edge.optional
                          ? 'cargo-optional is-optional'
                          : 'cargo-optional'
                      }
                    >
                      {edge.optional ? '可选依赖（optional）' : '非可选声明'}
                    </span>
                  </p>
                  <p className="cargo-edge-target">
                    <span className="cargo-cell-label">目标条件：</span>
                    <span
                      className={
                        edge.target === null
                          ? 'cargo-target is-empty'
                          : 'cargo-target'
                      }
                    >
                      {edge.target ?? '无目标条件'}
                    </span>
                  </p>
                  <div className="cargo-edge-sources">
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
                  </div>
                </li>
              ))}
            </ul>
          </div>
        ) : (
          <p className="operation-muted cargo-empty" role="status">
            当前种类没有内部依赖声明。
          </p>
        )}
        {edges.length > FOLD ? (
          <div className="cargo-fold">
            <Button
              size="sm"
              aria-expanded={allEdges}
              aria-controls="cargo-edges"
              onClick={() => setAllEdges((value) => !value)}
            >
              {allEdges
                ? `收起，只显示前 ${FOLD} 条`
                : `显示全部 ${edges.length} 条内部依赖声明`}
            </Button>
            <span className="cargo-fold__count">
              显示 {edgeRows.length} / {edges.length} 条
            </span>
          </div>
        ) : null}
      </div>
      <CargoPackageDetails item={selected} data={data} onOpen={onOpen} />
    </div>
  );
}
