import type { ArchitectureNode } from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';

const driftLabels = {
  present: '已声明 · 目录存在',
  missing: '已声明 · 目录缺失',
  undocumented: '目录存在 · 未声明',
  unverified: '未核验',
};

export function ArchitectureDirectoryEvidence({
  node,
  data,
  compact = false,
}: {
  node: ArchitectureNode;
  data: ProjectOperationsData;
  compact?: boolean;
}) {
  const observed = data.drift.find((item) => item.path === node.directory);
  const status =
    data.repository.status === 'ready' && observed
      ? observed.status
      : 'unverified';
  if (compact)
    return (
      <p className="architecture-step-evidence">
        目录：{node.directory ?? '未声明目录'} · {driftLabels[status]}
      </p>
    );
  return (
    <>
      <dl>
        <dt>声明目录</dt>
        <dd>{node.directory ?? '未声明目录'}</dd>
        <dt>目录核验</dt>
        <dd>{driftLabels[status]}</dd>
      </dl>
      {!observed ? (
        <p className="operation-muted">没有与声明目录精确匹配的观测记录。</p>
      ) : null}
    </>
  );
}
