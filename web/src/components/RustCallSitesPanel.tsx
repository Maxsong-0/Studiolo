import type { RustCallSitesData } from '../rustCallSites';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { RustCallSitesBoundary } from './RustCallSitesBoundary';
import { RustCallSitesView } from './RustCallSitesView';

const unavailable: Record<
  Exclude<RustCallSitesData['status'], 'ready'>,
  [string, string]
> = {
  disabled: [
    '尚未启用 Rust 源码读取',
    '仓库绑定没有启用源码白名单，本次未读取 Rust 源码。',
  ],
  unbound: [
    '源码仓库未绑定',
    '此项目没有匹配的仓库绑定，无法提供源码调用位置。',
  ],
  unavailable: [
    'Rust 源码暂不可用',
    '白名单文件缺失、不可读、不安全或读取期间变化，本次未发布部分证据。',
  ],
  invalid: ['Rust 源码证据无效', '路径或语法校验未通过，本次未发布部分证据。'],
  unsupported: [
    'Rust 源码超出支持范围',
    '文件数量、字节、语法结构预算或语法能力超限，本次未发布部分证据。',
  ],
};

export function RustCallSitesPanel({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const rust = data.rust_call_sites;
  const graph = rust?.status === 'ready' ? rust.graph : null;
  const reason = !rust
    ? ['暂无 Rust 调用位置数据', '当前服务未提供 Rust 源码调用位置证据。']
    : rust.status === 'ready'
      ? null
      : unavailable[rust.status];
  const observed = rust?.observed_at;
  return (
    <section
      className="operation-panel rust-panel"
      aria-labelledby="rust-call-sites-title"
    >
      <header className="operation-panel-heading">
        <div>
          <span className="operation-eyebrow">仓库源码语法证据</span>
          <h2 id="rust-call-sites-title">Rust 源码调用位置</h2>
        </div>
      </header>
      <p className="operation-muted">
        白名单文件 → 词法上下文 → 调用语法位置。未做编译器名字解析或类型检查；
        同名方法不能视为同一目标，调用语法也可能调用函数指针或闭包。
        这些位置不表示实际执行轨迹。
      </p>
      {graph && observed !== null && observed !== undefined ? (
        <>
          <p className="rust-observation">
            源码观测：
            <time dateTime={new Date(Number(observed) * 1000).toISOString()}>
              {new Date(Number(observed) * 1000).toLocaleString()}
            </time>
            ；文档声明使用快照 {data.generation}，各类证据没有共同事务观测时间。
          </p>
          {rust?.links ? (
            <details className="rust-link-batch">
              <summary>词法关联 · 输入批标识</summary>
              <code>{rust.links.batch_id}</code>
              <p>
                该标识说明输入内容身份，不是文件系统快照或新鲜度证明。
                前端没有源码正文，无法独立重算内容哈希；它与文档代次和 Cargo
                观测独立。
              </p>
            </details>
          ) : (
            <p className="operation-muted">
              当前服务未提供词法关联；保留调用位置，目标未解析。
            </p>
          )}
          <p className="operation-muted">
            仅覆盖下列 {graph.files.length} 个显式白名单文件，共{' '}
            {graph.calls.length} 处调用位置。
            仓库源码位置以文本显示；没有读取源码正文或调用参数的入口。
          </p>
          {graph.calls.length === 0 ? (
            <p className="operation-muted">
              零结果只表示白名单中没有提取到调用位置，不代表全仓库没有调用。
            </p>
          ) : null}
          <RustCallSitesBoundary
            key={`${data.project}:${data.generation}:${observed}:${rust?.links?.batch_id ?? 'legacy'}`}
            graph={graph}
            data={data}
            onOpen={onOpen}
          >
            <RustCallSitesView graph={graph} data={data} onOpen={onOpen} />
          </RustCallSitesBoundary>
        </>
      ) : (
        <div className="operation-unavailable" role="status">
          <h3>{reason?.[0]}</h3>
          <p>{reason?.[1]}</p>
        </div>
      )}
      {rust?.diagnostics.length ? (
        <details className="diagnostics operation-diagnostics" open>
          <summary>源码观测说明（{rust.diagnostics.length}）</summary>
          <ul>
            {rust.diagnostics.map((message, index) => (
              <li key={index}>{message}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}
