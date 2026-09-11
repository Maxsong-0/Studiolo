import { useState } from 'react';
import type {
  ArchitectureData,
  ArchitectureFlow,
  ArchitectureNode,
} from '../architecture';
import type { OpenSource } from '../types';
import type { ProjectOperationsData } from '../projectOperations';
import { ArchitectureDirectoryEvidence } from './ArchitectureDirectoryEvidence';

export function ArchitectureFlows({
  architecture,
  data,
  onOpen,
}: {
  architecture: ArchitectureData;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const flow =
    architecture.flows.find((item) => item.id === selection) ??
    architecture.flows[0];
  if (!flow)
    return (
      <p className="operation-unavailable" role="status">
        文档尚未声明设计路径。已有模块关系仍可查看。
      </p>
    );
  return (
    <div className="architecture-flows">
      <div className="architecture-filter">
        <label htmlFor="architecture-flow">设计路径</label>
        <select
          id="architecture-flow"
          value={flow.id}
          onChange={(event) => setSelection(event.target.value)}
        >
          {architecture.flows.map((item) => (
            <option key={item.id} value={item.id}>
              {item.label}
            </option>
          ))}
        </select>
      </div>
      <ArchitectureSteps
        key={flow.id}
        flow={flow}
        nodes={architecture.nodes}
        data={data}
        onOpen={onOpen}
      />
    </div>
  );
}

function ArchitectureSteps({
  flow,
  nodes,
  data,
  onOpen,
}: {
  flow: ArchitectureFlow;
  nodes: ArchitectureNode[];
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [position, setPosition] = useState(0);
  const step = flow.steps[position];
  const node = nodes.find((item) => item.id === step.node)!;
  return (
    <div className="architecture-steps-layout">
      <ol className="architecture-steps" aria-label="设计步骤">
        {flow.steps.map((item, index) => (
          <li key={item.order}>
            <button
              aria-current={index === position ? 'step' : undefined}
              onClick={() => setPosition(index)}
            >
              <span>{item.order}</span>
              <strong>
                {nodes.find((entry) => entry.id === item.node)!.label}
              </strong>
            </button>
          </li>
        ))}
      </ol>
      <div className="architecture-step-detail">
        <div aria-live="polite" aria-label="当前设计步骤">
          <p className="operation-eyebrow">
            设计顺序 · 第 {position + 1} / {flow.steps.length} 步
          </p>
          <h3>{node.label}</h3>
          <p>{step.description || '未提供步骤说明。'}</p>
          <ArchitectureDirectoryEvidence node={node} data={data} compact />
          <p className="operation-source-location">
            {step.source.path} · 第 {step.source.line} 行
          </p>
          <button className="text-button" onClick={() => onOpen(step.source)}>
            查看步骤声明原文
          </button>
        </div>
        <div className="architecture-step-controls">
          <button
            className="outline-button"
            aria-disabled={position === 0}
            onClick={() => setPosition((current) => Math.max(0, current - 1))}
          >
            前一步
          </button>
          <button
            className="outline-button"
            aria-disabled={position === flow.steps.length - 1}
            onClick={() =>
              setPosition((current) =>
                Math.min(flow.steps.length - 1, current + 1),
              )
            }
          >
            后一步
          </button>
        </div>
      </div>
    </div>
  );
}
