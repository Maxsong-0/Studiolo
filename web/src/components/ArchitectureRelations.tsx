import { useState } from 'react';
import { nodeKinds, relationKinds } from '../architecture';
import type { ArchitectureData, ArchitectureRelation } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { ArchitectureNodeDetails } from './ArchitectureNodeDetails';
import { Icon } from './Icon';

export function ArchitectureRelations({
  architecture,
  data,
  onOpen,
}: {
  architecture: ArchitectureData;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const [kind, setKind] = useState<'all' | ArchitectureRelation['kind']>('all');
  const nodes = new Map(architecture.nodes.map((node) => [node.id, node]));
  const selected = nodes.get(selection) ?? architecture.nodes[0];
  const relations = architecture.relations.filter(
    (relation) => kind === 'all' || relation.kind === kind,
  );
  return (
    <div className="architecture-relations-layout">
      <div className="architecture-relations-main">
        <div
          className="architecture-node-list"
          role="group"
          aria-label="架构节点"
        >
          {architecture.nodes.map((node) => (
            <button
              className="architecture-node"
              key={node.id}
              aria-pressed={node.id === selected.id}
              onClick={() => setSelection(node.id)}
              aria-label={`选择架构节点 ${node.label}`}
            >
              <span className="operation-eyebrow">{nodeKinds[node.kind]}</span>
              <strong>{node.label}</strong>
            </button>
          ))}
        </div>
        <div className="architecture-filter">
          <label htmlFor="architecture-relation-kind">关系类型</label>
          <select
            id="architecture-relation-kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as typeof kind)}
          >
            <option value="all">全部关系</option>
            <option value="dependency">依赖声明</option>
            <option value="data_flow">数据流声明</option>
          </select>
          <span className="operation-muted">{relations.length} 条文档声明</span>
        </div>
        {relations.length ? (
          <ul className="architecture-edge-list" aria-label="架构关系声明">
            {relations.map((relation, index) => (
              <li
                key={`${relation.from}:${relation.to}:${relation.kind}:${index}`}
              >
                <div className="architecture-edge-endpoints">
                  <button
                    onClick={() => setSelection(relation.from)}
                    aria-label={`选择关系起点 ${nodes.get(relation.from)!.label}`}
                  >
                    {nodes.get(relation.from)!.label}
                  </button>
                  <Icon name="arrow" />
                  <button
                    onClick={() => setSelection(relation.to)}
                    aria-label={`选择关系终点 ${nodes.get(relation.to)!.label}`}
                  >
                    {nodes.get(relation.to)!.label}
                  </button>
                </div>
                <p>
                  <span className="architecture-relation-kind">
                    {relationKinds[relation.kind]}
                  </span>
                  {relation.label || '未提供关系说明'}
                </p>
                <button
                  className="text-button"
                  onClick={() => onOpen(relation.source)}
                  aria-label={`${nodes.get(relation.from)!.label}到${nodes.get(relation.to)!.label}，查看${relationKinds[relation.kind]}来源`}
                >
                  查看关系原文
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="operation-muted" role="status">
            当前类型没有关系声明。
          </p>
        )}
      </div>
      <ArchitectureNodeDetails node={selected} data={data} onOpen={onOpen} />
    </div>
  );
}
