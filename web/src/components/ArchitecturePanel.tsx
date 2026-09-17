import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { ArchitectureBoundary } from './ArchitectureBoundary';
import { ArchitectureView } from './ArchitectureView';
import { Icon } from './Icon';

export function ArchitecturePanel({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const architecture = data.architecture;
  return (
    <section
      className="operation-panel architecture-panel"
      aria-labelledby="architecture-title"
    >
      <header className="operation-panel-heading">
        <div>
          <span className="operation-eyebrow">架构文档</span>
          <h2 id="architecture-title">模块关系与设计路径</h2>
        </div>
        {architecture?.source ? (
          <button
            className="outline-button"
            onClick={() => onOpen(architecture.source!)}
          >
            <Icon name="document" />
            查看架构声明
          </button>
        ) : null}
      </header>
      <p className="operation-muted">
        关系来自文档声明，不代表实测调用或已实现的数据流。
      </p>
      {architecture?.status === 'ready' ? (
        <ArchitectureBoundary
          key={`${data.project}:${data.generation}`}
          data={architecture}
          onOpen={onOpen}
        >
          <ArchitectureView
            architecture={architecture}
            data={data}
            onOpen={onOpen}
          />
        </ArchitectureBoundary>
      ) : (
        <div className="operation-unavailable" role="status">
          <h3>
            {architecture?.status === 'invalid'
              ? '架构声明不完整或格式无效'
              : '暂无可用的架构关系声明'}
          </h3>
          <p>
            此项目尚无可展示的模块关系与设计路径。请核对项目运作文档中的节点、关系和设计路径声明，再刷新工作台。
          </p>
        </div>
      )}
      {architecture?.diagnostics.length ? (
        <details className="diagnostics operation-diagnostics" open>
          <summary>架构声明说明（{architecture.diagnostics.length}）</summary>
          <ul>
            {architecture.diagnostics.map((message, index) => (
              <li key={index}>{message}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}
