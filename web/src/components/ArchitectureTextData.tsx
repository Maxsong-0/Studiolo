import { nodeKinds, relationKinds } from '../architecture';
import type { ArchitectureData } from '../architecture';
import type { OpenSource } from '../types';

export function ArchitectureTextData({
  data,
  onOpen,
}: {
  data: ArchitectureData;
  onOpen: OpenSource;
}) {
  const labels = new Map(data.nodes.map((node) => [node.id, node.label]));
  return (
    <div
      className="architecture-text-data"
      role="region"
      aria-label="架构声明文本数据"
    >
      <h4>节点声明</h4>
      <ul>
        {data.nodes.map((node) => (
          <li key={node.id}>
            <strong>{node.label}</strong> · {nodeKinds[node.kind]}
            <p>{node.description}</p>
            <p>
              声明目录：{node.directory ?? '未声明目录'} ·
              文本视图未展示目录核验结果
            </p>
            <button
              className="text-button"
              onClick={() => onOpen(node.source)}
              aria-label={`${node.label}，从文本查看节点来源`}
            >
              查看节点原文
            </button>
          </li>
        ))}
      </ul>
      <h4>关系声明</h4>
      <ul>
        {data.relations.map((relation, index) => (
          <li key={index}>
            <strong>
              {labels.get(relation.from)} → {labels.get(relation.to)}
            </strong>
            <p>
              {relationKinds[relation.kind]} · {relation.label}
            </p>
            <button
              className="text-button"
              onClick={() => onOpen(relation.source)}
            >
              查看关系原文
            </button>
          </li>
        ))}
      </ul>
      <h4>设计路径声明</h4>
      <p className="operation-muted">
        步骤仅表示设计顺序，不执行操作，也不表示实时任务进度。
      </p>
      {data.flows.map((flow) => (
        <section key={flow.id} aria-label={flow.label}>
          <h5>{flow.label}</h5>
          <ol>
            {flow.steps.map((step) => (
              <li key={step.order}>
                <strong>{labels.get(step.node)}</strong>
                <p>{step.description}</p>
                <button
                  className="text-button"
                  onClick={() => onOpen(step.source)}
                >
                  查看步骤原文
                </button>
              </li>
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}
