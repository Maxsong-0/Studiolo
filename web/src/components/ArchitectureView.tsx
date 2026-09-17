import { useId, useState } from 'react';
import { motion } from 'motion/react';
import type { ArchitectureData } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
import { ArchitectureRelations } from './ArchitectureRelations';
import { ArchitectureFlows } from './ArchitectureFlows';

const modes = [
  { id: 'relations', label: '模块关系' },
  { id: 'flows', label: '设计路径' },
] as const;

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
  const selectionId = useId();
  const reduced = useReducedMotionPreference();
  return (
    <>
      <div
        className="document-modes architecture-modes"
        role="group"
        aria-label="架构浏览方式"
      >
        {modes.map((item) => (
          <button
            key={item.id}
            aria-pressed={mode === item.id}
            onClick={() => setMode(item.id)}
          >
            {mode === item.id ? (
              <motion.span
                key={reduced ? 'reduced' : 'animated'}
                className="document-mode-selected"
                aria-hidden="true"
                layoutId={reduced ? undefined : selectionId}
                initial={false}
                transition={{ type: 'spring', stiffness: 500, damping: 45 }}
              />
            ) : null}
            {item.label}
          </button>
        ))}
      </div>
      <div hidden={mode !== 'relations'}>
        <ArchitectureRelations
          architecture={architecture}
          data={data}
          onOpen={onOpen}
        />
      </div>
      <div hidden={mode !== 'flows'}>
        <p className="architecture-design-note">
          下列步骤仅表示文档声明的设计顺序。使用前后步浏览，不执行操作，也不表示实时任务进度。
        </p>
        <ArchitectureFlows
          architecture={architecture}
          data={data}
          onOpen={onOpen}
        />
      </div>
    </>
  );
}
