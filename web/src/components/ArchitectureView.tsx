import { useState } from 'react';
import type { ArchitectureData } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Cell } from '../ui';
import { ArchitectureRelations } from './ArchitectureRelations';
import { ArchitectureFlows } from './ArchitectureFlows';
import '../styles/architecture.css';

const modes = [
  { id: 'relations', label: '模块关系' },
  { id: 'flows', label: '设计路径' },
] as const;

/**
 * 架构的两种浏览方式：模块关系（关系图＋关系明细＋所选节点）与设计路径（步骤链＋当前步骤）。
 * 两个面板都挂着、只切 hidden，切回来时各自的选择与步骤原样保留；选中项黑底反白，不做弹簧过场。
 */
export function ArchitectureView({
  architecture,
  data,
  onOpen,
}: {
  architecture: ArchitectureData;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [mode, setMode] = useState<'relations' | 'flows'>('relations');
  return (
    <>
      <Cell span={12} as="div">
        <div
          className="architecture-modes"
          role="group"
          aria-label="架构浏览方式"
        >
          {modes.map((item) => (
            <button
              key={item.id}
              type="button"
              className="architecture-mode"
              aria-pressed={mode === item.id}
              onClick={() => setMode(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
      </Cell>
      <div className="architecture-pane" hidden={mode !== 'relations'}>
        <ArchitectureRelations
          architecture={architecture}
          data={data}
          onOpen={onOpen}
        />
      </div>
      <div className="architecture-pane" hidden={mode !== 'flows'}>
        <Cell span={12} as="div">
          <p className="architecture-design-note">
            下列步骤仅表示文档声明的设计顺序。使用前后步浏览，不执行操作，也不表示实时任务进度。
          </p>
        </Cell>
        <ArchitectureFlows
          architecture={architecture}
          data={data}
          onOpen={onOpen}
        />
      </div>
    </>
  );
}
