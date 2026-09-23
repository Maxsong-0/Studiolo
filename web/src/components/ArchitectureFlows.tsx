import { useState } from 'react';
import type { ArchitectureData } from '../architecture';
import type { OpenSource } from '../types';
import type { ProjectOperationsData } from '../projectOperations';
import { Button, Cell, CircledNumber } from '../ui';
import { ArchitectureDirectoryEvidence } from './ArchitectureDirectoryEvidence';
import { Icon } from './Icon';
import '../styles/projects.css';
import '../styles/architecture.css';

/**
 * 设计路径：左 4 列选流程、自上而下的步骤链（圈号＋节点框，框间 1.3px 线与箭头），
 * 右 8 列当前步骤与前后步。步骤只是文档声明的设计顺序，不执行、不代表实时进度。
 * 当前步骤记在这一层：换流程时回到第 1 步，切到模块关系再切回来时原样保留，选择框不重挂。
 */
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
  const [position, setPosition] = useState(0);
  const flow =
    architecture.flows.find((item) => item.id === selection) ??
    architecture.flows[0];
  if (!flow)
    return (
      <Cell span={12} as="div">
        <p className="operation-muted" role="status">
          文档尚未声明设计路径。已有模块关系仍可查看。
        </p>
      </Cell>
    );
  const labels = new Map(
    architecture.nodes.map((node) => [node.id, node.label]),
  );
  const last = flow.steps.length - 1;
  const current = Math.min(position, last);
  const step = flow.steps[current];
  const node = architecture.nodes.find((item) => item.id === step.node)!;
  return (
    <>
      <Cell span={4} as="div">
        <div className="architecture-filter">
          <label htmlFor="architecture-flow">设计路径</label>
          <span className="project-field project-field--select">
            <select
              id="architecture-flow"
              value={flow.id}
              onChange={(event) => {
                setSelection(event.target.value);
                setPosition(0);
              }}
            >
              {architecture.flows.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
            <Icon name="chevron" />
          </span>
        </div>
        <ol className="architecture-steps" aria-label="设计步骤">
          {flow.steps.map((item, index) => (
            <li key={item.order}>
              <button
                type="button"
                aria-current={index === current ? 'step' : undefined}
                onClick={() => setPosition(index)}
              >
                <CircledNumber n={item.order} />{' '}
                <strong>{labels.get(item.node)}</strong>
              </button>
            </li>
          ))}
        </ol>
      </Cell>
      <Cell span={8} as="div" className="architecture-step-detail">
        <div
          className="architecture-step-current"
          aria-live="polite"
          aria-label="当前设计步骤"
        >
          <p className="operation-label">
            设计顺序 · 第 {current + 1} / {flow.steps.length} 步
          </p>
          <h3>{node.label}</h3>
          <p>{step.description || '未提供步骤说明。'}</p>
          <ArchitectureDirectoryEvidence node={node} data={data} compact />
          <p className="operation-source-location">
            {step.source.path} · 第 {step.source.line} 行
          </p>
          <Button variant="text" size="sm" onClick={() => onOpen(step.source)}>
            查看步骤声明原文
          </Button>
        </div>
        <div className="architecture-step-controls">
          <Button
            aria-disabled={current === 0}
            onClick={() => setPosition((value) => Math.max(0, value - 1))}
          >
            前一步
          </Button>
          <Button
            aria-disabled={current === last}
            onClick={() => setPosition((value) => Math.min(last, value + 1))}
          >
            后一步
          </Button>
        </div>
      </Cell>
    </>
  );
}
