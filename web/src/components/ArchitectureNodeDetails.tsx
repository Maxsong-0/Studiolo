import { nodeKinds } from '../architecture';
import type { ArchitectureNode } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { ArchitectureDirectoryEvidence } from './ArchitectureDirectoryEvidence';

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
    <aside
      className="architecture-node-detail"
      aria-label="所选架构节点"
      aria-live="polite"
    >
      <span className="operation-eyebrow">
        {nodeKinds[node.kind]} · 文档声明
      </span>
      <h3>{node.label}</h3>
      <p>{node.description || '未提供职责说明。'}</p>
      <ArchitectureDirectoryEvidence node={node} data={data} />
      <p className="operation-muted">
        目录状态仅反映目录证据，不能证明模块调用或数据流已经实现。
      </p>
      <p className="operation-source-location">
        {node.source.path} · 第 {node.source.line} 行
      </p>
      <button className="text-button" onClick={() => onOpen(node.source)}>
        查看节点声明原文
      </button>
    </aside>
  );
}
