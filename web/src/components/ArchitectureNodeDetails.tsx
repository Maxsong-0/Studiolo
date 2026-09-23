import { nodeKinds } from '../architecture';
import type { ArchitectureNode } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Cell } from '../ui';
import { ArchitectureDirectoryEvidence } from './ArchitectureDirectoryEvidence';
import '../styles/operations.css';
import '../styles/architecture.css';

/** 所选节点（关系明细右侧 4 列）：类型、职责、目录证据与节点声明原文。 */
export function ArchitectureNodeDetails({
  node,
  data,
  onOpen,
}: {
  node: ArchitectureNode;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  return (
    <Cell
      span={4}
      as="aside"
      className="operation-selection architecture-node-detail"
      aria-label="所选架构节点"
      aria-live="polite"
    >
      <span className="operation-label">{nodeKinds[node.kind]} · 文档声明</span>
      <h3>{node.label}</h3>
      <p>{node.description || '未提供职责说明。'}</p>
      <ArchitectureDirectoryEvidence node={node} data={data} />
      <p className="operation-muted">
        目录状态仅反映目录证据，不能证明模块调用或数据流已经实现。
      </p>
      <p className="operation-source-location">
        {node.source.path} · 第 {node.source.line} 行
      </p>
      <Button variant="text" size="sm" onClick={() => onOpen(node.source)}>
        查看节点声明原文
      </Button>
    </Cell>
  );
}
