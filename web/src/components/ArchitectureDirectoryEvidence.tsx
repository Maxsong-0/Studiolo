import type { ArchitectureNode } from '../architecture';
import type {
  DirectoryDrift,
  ProjectOperationsData,
} from '../projectOperations';
import { StatusMark, type StatusState } from '../ui';
import '../styles/operations.css';
import '../styles/architecture.css';

type DriftStatus = DirectoryDrift['status'];

const driftLabels: Record<DriftStatus, string> = {
  present: '已声明 · 目录存在',
  missing: '已声明 · 目录缺失',
  undocumented: '目录存在 · 未声明',
  unverified: '未核验',
};

/** 与目录漂移同一套记号：红色只给缺失与未声明。 */
const driftMarks: Record<DriftStatus, StatusState> = {
  present: 'ok',
  missing: 'alert',
  undocumented: 'alert',
  unverified: 'dormant',
};

/** 节点声明目录的核验证据：只认与漂移结果精确匹配的目录；仓库未就绪一律未核验。 */
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
  const mark = (
    <StatusMark state={driftMarks[status]}>{driftLabels[status]}</StatusMark>
  );
  if (compact)
    return (
      <p className="architecture-evidence architecture-evidence--step">
        目录：{node.directory ?? '未声明目录'} · {mark}
      </p>
    );
  return (
    <>
      <dl className="architecture-evidence">
        <dt>声明目录</dt>
        <dd>{node.directory ?? '未声明目录'}</dd>
        <dt>目录核验</dt>
        <dd>{mark}</dd>
      </dl>
      {!observed ? (
        <p className="operation-muted">没有与声明目录精确匹配的观测记录。</p>
      ) : null}
    </>
  );
}
