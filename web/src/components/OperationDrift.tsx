import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';

const statusLabels = {
  present: '已声明 · 目录存在',
  missing: '已声明 · 目录缺失',
  undocumented: '目录存在 · 未声明',
  unverified: '未核验',
};

export function OperationDrift({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const observed = data.repository.observed_at;
  const verified = data.repository.status === 'ready';
  const status =
    data.repository.status === 'unbound'
      ? '仓库未绑定 · 未核验'
      : verified
        ? '本次目录列举完成'
        : '仓库不可用 · 未核验';
  return (
    <section
      className="operation-panel"
      aria-labelledby="operation-drift-title"
    >
      <header className="operation-panel-heading">
        <div>
          <span className="operation-eyebrow">声明与实际目录</span>
          <h2 id="operation-drift-title">双向目录漂移</h2>
        </div>
        <span className="operation-status">{status}</span>
      </header>
      <p className="operation-muted">
        {data.repository.status === 'unbound'
          ? '尚未配置此项目的本地仓库绑定。以下声明没有经过实际目录核验。'
          : '仅比较下列父目录的直属目录；没有观测证据的声明保持未核验。此结果不代表全仓库检查通过。'}
      </p>
      <div className="operation-scope">
        <strong>比较父目录</strong>
        {data.comparison_parents.length ? (
          <ul>
            {data.comparison_parents.map((path) => (
              <li key={path}>
                <code>{path || '.'}</code>
              </li>
            ))}
          </ul>
        ) : (
          <span>暂无可比较的父目录</span>
        )}
      </div>
      <p className="operation-observed">
        仓库观测：
        {observed !== null ? (
          <time dateTime={new Date(Number(observed) * 1000).toISOString()}>
            {new Date(Number(observed) * 1000).toLocaleString()}
          </time>
        ) : (
          '暂无可信观测时间'
        )}
        <span>
          仓库目录按本次读取独立观测；文档声明绑定快照 {data.generation}。
        </span>
      </p>
      {data.drift.length ? (
        <ul className="operation-drift-list" aria-label="目录比对结果">
          {data.drift.map((item, index) => {
            const currentStatus = verified ? item.status : 'unverified';
            return (
              <li key={`${item.path}:${index}`}>
                <code>{item.path}</code>
                <span className={`operation-drift-status is-${currentStatus}`}>
                  {statusLabels[currentStatus]}
                </span>
                {item.source ? (
                  <button
                    className="text-button"
                    onClick={() => onOpen(item.source!)}
                    aria-label={`${item.path}，查看目录声明来源`}
                  >
                    查看声明
                  </button>
                ) : (
                  <span className="operation-muted">无声明来源</span>
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="operation-muted">
          暂无目录比对结果。请先核对结构声明、仓库绑定与比较范围。
        </p>
      )}
    </section>
  );
}
