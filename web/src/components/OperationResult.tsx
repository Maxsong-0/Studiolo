import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { OperationDiagram } from './OperationDiagram';
import { OperationDiagramBoundary } from './OperationDiagramBoundary';
import { OperationDrift } from './OperationDrift';
import { ArchitecturePanel } from './ArchitecturePanel';
import { Icon } from './Icon';

export function OperationResult({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const hasDiagram =
    data.declaration_status === 'ready' && data.directories.length > 0;
  return (
    <>
      <section
        className="operation-panel"
        aria-labelledby="operation-diagram-title"
      >
        <header className="operation-panel-heading">
          <div>
            <span className="operation-eyebrow">项目结构</span>
            <h2 id="operation-diagram-title">目录与模块职责</h2>
          </div>
          {data.declaration_source ? (
            <button
              className="outline-button"
              onClick={() => onOpen(data.declaration_source!)}
            >
              <Icon name="document" />
              查看结构声明
            </button>
          ) : null}
        </header>
        {hasDiagram ? (
          <>
            <p className="operation-muted">
              选择目录查看声明来源。连线仅表示目录层级，职责来自结构声明。
            </p>
            <OperationDiagramBoundary
              key={`${data.project}:${data.generation}`}
              directories={data.directories}
              onOpen={onOpen}
            >
              <OperationDiagram
                directories={data.directories}
                onOpen={onOpen}
              />
            </OperationDiagramBoundary>
          </>
        ) : (
          <div className="operation-unavailable" role="status">
            <h3>
              {data.declaration_status === 'invalid'
                ? '结构声明不完整或格式无效'
                : '暂无可用的结构声明'}
            </h3>
            <p>
              当前无法生成目录与模块职责图。请在项目架构文档中补齐受支持的结构声明，再刷新工作台。
            </p>
            <p>已有文档仍可通过“文档”视图阅读。</p>
          </div>
        )}
        {data.diagnostics.length ? (
          <details className="diagnostics operation-diagnostics" open>
            <summary>声明与核验说明（{data.diagnostics.length}）</summary>
            <ul>
              {data.diagnostics.map((message, index) => (
                <li key={index}>{message}</li>
              ))}
            </ul>
          </details>
        ) : null}
      </section>
      <OperationDrift data={data} onOpen={onOpen} />
      <ArchitecturePanel data={data} onOpen={onOpen} />
    </>
  );
}
