import type { BoardItem } from './boardModel';

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
        <ul aria-label="暂存候选指针">
          {item.candidateStages.map((stage, i) => (
            <li key={`${stage.pointer.id}-${i}`}>
              <p>
                Attempt {stage.attemptId} · fence {stage.fence} · {stage.reason}
              </p>
              <code>id: {stage.pointer.id}</code>
              <br />
              <code>sha256: {stage.pointer.sha256}</code>
            </li>
          ))}
        </ul>
      ) : null}
      {item.candidateReceipt ? (
        <>
          <p>
            候选清单：<code>{item.candidateReceipt.manifestPath}</code>
          </p>
          <p>
            清单 revision：<code>{item.candidateReceipt.manifestRevision}</code>
          </p>
          <ul aria-label="候选产物">
            {item.candidateReceipt.artifacts.map((p, i) => (
              <li key={`${p.path}-${i}`}>
                <code>{p.path}</code>
                <br />
                <code>revision: {p.revision}</code>
                <br />
                <code>sha256: {p.sha256}</code>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </details>
  );
}
