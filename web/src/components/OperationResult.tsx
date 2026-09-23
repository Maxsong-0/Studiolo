import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Cell, CellHeader, EmptyState, GridFrame } from '../ui';
import { OperationDiagram } from './OperationDiagram';
import { OperationDiagramBoundary } from './OperationDiagramBoundary';
import { OperationDrift } from './OperationDrift';
import { ArchitecturePanel } from './ArchitecturePanel';
import { CargoDependencyPanel } from './CargoDependencyPanel';
import { RustCallSitesPanel } from './RustCallSitesPanel';
import '../styles/operations.css';

/**
 * 运作页的各块，全部是运作网格里的整行格子（12 列）：
 * 目录与模块职责（块内：标题 12 · 目录树 8 · 所选目录 4）、目录漂移、架构关系与流程、
 * Cargo 清单依赖、Rust 源码调用位置。后两块的组件归代码证据线，这里只把它们摆进格子。
 */
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
      <Cell
        span={12}
        flush
        as="section"
        className="operation-block"
        aria-labelledby="operation-diagram-title"
      >
        <GridFrame className="operation-subgrid">
          <Cell span={12}>
            <CellHeader
              title={<span id="operation-diagram-title">目录与模块职责</span>}
              meta={
                hasDiagram ? `${data.directories.length} 个目录` : undefined
              }
            />
            {hasDiagram ? (
              <p className="operation-note">
                选择目录查看声明来源。连线仅表示目录层级，职责来自结构声明。
              </p>
            ) : null}
          </Cell>
          {hasDiagram ? (
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
          ) : (
            <Cell span={12}>
              <div role="status">
                <EmptyState
                  title={
                    data.declaration_status === 'invalid'
                      ? '结构声明不完整或格式无效'
                      : '暂无可用的结构声明'
                  }
                >
                  <p>
                    当前无法生成目录与模块职责图。请在项目架构文档中补齐受支持的结构声明，再刷新工作台。
                  </p>
                  <p>已有文档仍可通过“文档”视图阅读。</p>
                </EmptyState>
              </div>
            </Cell>
          )}
          {data.diagnostics.length ? (
            <Cell span={12}>
              <details className="operation-diagnostics" open>
                <summary>声明与核验说明（{data.diagnostics.length}）</summary>
                <ul>
                  {data.diagnostics.map((message, index) => (
                    <li key={index}>{message}</li>
                  ))}
                </ul>
              </details>
            </Cell>
          ) : null}
        </GridFrame>
      </Cell>
      <OperationDrift data={data} onOpen={onOpen} />
      <ArchitecturePanel data={data} onOpen={onOpen} />
      <Cell span={12} as="div" className="operation-evidence">
        <CargoDependencyPanel data={data} onOpen={onOpen} />
      </Cell>
      <Cell span={12} as="div" className="operation-evidence">
        <RustCallSitesPanel data={data} onOpen={onOpen} />
      </Cell>
    </>
  );
}
