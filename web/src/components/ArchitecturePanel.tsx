import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Cell, CellHeader, EmptyState, GridFrame } from '../ui';
import { ArchitectureBoundary } from './ArchitectureBoundary';
import { ArchitectureView } from './ArchitectureView';
import { Icon } from './Icon';
import '../styles/operations.css';
import '../styles/architecture.css';

/**
 * 架构关系与流程：运作网格里的整行格子，块内再分格（标题 12 · 浏览方式 12 ·
 * 模块关系：关系图 12、关系明细 8、所选节点 4；设计路径：说明 12、步骤 4、当前步骤 8）。
 */
export function ArchitecturePanel({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const architecture = data.architecture;
  return (
    <Cell
      span={12}
      flush
      as="section"
      className="operation-block architecture-panel"
      aria-labelledby="architecture-title"
    >
      <GridFrame className="operation-subgrid">
        <Cell span={12}>
          <CellHeader
            title={<span id="architecture-title">模块关系与设计路径</span>}
            action={
              architecture?.source ? (
                <Button size="sm" onClick={() => onOpen(architecture.source!)}>
                  <Icon name="document" />
                  查看架构声明
                </Button>
              ) : undefined
            }
          />
          <p className="operation-note">
            关系来自文档声明，不代表实测调用或已实现的数据流。
          </p>
        </Cell>
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
          <Cell span={12}>
            <div role="status">
              <EmptyState
                title={
                  architecture?.status === 'invalid'
                    ? '架构声明不完整或格式无效'
                    : '暂无可用的架构关系声明'
                }
              >
                <p>
                  此项目尚无可展示的模块关系与设计路径。请核对项目运作文档中的节点、关系和设计路径声明，再刷新工作台。
                </p>
              </EmptyState>
            </div>
          </Cell>
        )}
        {architecture?.diagnostics.length ? (
          <Cell span={12}>
            <details className="operation-diagnostics" open>
              <summary>
                架构声明说明（{architecture.diagnostics.length}）
              </summary>
              <ul>
                {architecture.diagnostics.map((message, index) => (
                  <li key={index}>{message}</li>
                ))}
              </ul>
            </details>
          </Cell>
        ) : null}
      </GridFrame>
    </Cell>
  );
}
