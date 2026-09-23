import type { BoardItem } from './boardModel';
import { shortHash } from './executionView';
import { StatusMark } from './ui';
import './styles/execution.css';

/** 候选指针只带路径、revision 与 SHA-256：大小接口没给，显示「—」并写明原因。 */
const NO_SIZE = '候选指针只含路径、revision 与 SHA-256，接口没有给出产物大小。';

/**
 * 候选证据（协作 › 执行与任务与回执共用）：暂存记录与已提交的候选产物逐条列出，
 * 状态方块＋等宽摘要前 8 位（完整值在 title）＋大小。是否通过以逐项验收和收尾回执为准。
 */
export function CandidateDetails({ item }: { item: BoardItem }) {
  if (!item.candidateReceipt && !item.candidateStages.length) return null;
  return (
    <details className="board-receipt execution-candidates">
      <summary>
        候选证据 · {item.candidateStages.length} 个暂存记录
        {item.candidateReceipt ? ' · 已提交候选批次' : ''}
      </summary>
      <p>这些指针用于审阅；是否通过以逐项验收和收尾回执为准。</p>
      {item.candidateStages.length ? (
        <ul aria-label="暂存候选指针" className="execution-artifacts">
          {item.candidateStages.map((stage, i) => (
            <li key={`${stage.pointer.id}-${i}`} className="execution-artifact">
              <StatusMark state="pending" className="execution-artifact__state">
                暂存
              </StatusMark>
              <span className="execution-artifact__main">{stage.reason}</span>
              <span className="execution-artifact__meta">
                Attempt <code>{stage.attemptId}</code> · fence{' '}
                <code>{stage.fence}</code>
              </span>
              <span className="execution-artifact__meta">
                id{' '}
                <code title={stage.pointer.id}>
                  {shortHash(stage.pointer.id)}
                </code>{' '}
                · sha{' '}
                <code title={stage.pointer.sha256}>
                  {shortHash(stage.pointer.sha256)}
                </code>{' '}
                · 大小 <span title={NO_SIZE}>—</span>
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {item.candidateReceipt ? (
        <>
          <dl className="execution-facts execution-manifest">
            <div>
              <dt>候选清单</dt>
              <dd>
                <code>{item.candidateReceipt.manifestPath}</code>
              </dd>
            </div>
            <div>
              <dt>清单 revision</dt>
              <dd>
                <code>{item.candidateReceipt.manifestRevision}</code>
              </dd>
            </div>
          </dl>
          <ul aria-label="候选产物" className="execution-artifacts">
            {item.candidateReceipt.artifacts.map((p, i) => (
              <li key={`${p.path}-${i}`} className="execution-artifact">
                <StatusMark state="ok" className="execution-artifact__state">
                  已封存
                </StatusMark>
                <code className="execution-artifact__main">{p.path}</code>
                <span className="execution-artifact__meta">
                  sha <code title={p.sha256}>{shortHash(p.sha256)}</code> · 大小{' '}
                  <span title={NO_SIZE}>—</span> · revision{' '}
                  <code title={p.revision}>{shortHash(p.revision, 12)}</code>
                </span>
              </li>
            ))}
          </ul>
        </>
      ) : null}
      <p className="execution-hint">大小一栏的「—」：{NO_SIZE}</p>
    </details>
  );
}
