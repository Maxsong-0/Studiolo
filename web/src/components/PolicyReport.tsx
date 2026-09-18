import type { Report, Expansion, DiffEntry } from '../policyModel';
import { shortHash } from '../policyModel';

function ExpansionList({
  items,
  kind,
}: {
  items: Expansion[];
  kind: 'expansion' | 'narrowing';
}) {
  if (!items.length) return null;
  return (
    <ul
      className={`policy-${kind}s`}
      aria-label={kind === 'expansion' ? '权限扩大项' : '权限收窄项'}
    >
      {items.map((item, index) => (
        <li key={`${item.path}-${index}`}>
          <code>{item.path}</code>
          {item.before !== null ? <span> 原：{item.before}</span> : null}
          {item.after !== null ? <span> 改为：{item.after}</span> : null}
          <em>{item.note}</em>
        </li>
      ))}
    </ul>
  );
}

const OP_LABEL: Record<DiffEntry['op'], string> = {
  added: '＋',
  removed: '－',
  changed: '～',
};

/** 校验报告：问题、扩大项标红、收窄项、diff；不渲染 Markdown，正文一律当文本。 */
export function PolicyReport({ report }: { report: Report }) {
  return (
    <div
      className={`policy-report ${report.ok ? 'is-ok' : 'is-rejected'}`}
      aria-label="校验报告"
    >
      <p className="policy-report-summary">
        <strong>{report.ok ? '校验通过' : '校验未通过'}</strong>
        <span>
          基于 effective{' '}
          <code title={report.base_effective_hash}>
            {shortHash(report.base_effective_hash)}
          </code>
          {report.current_version !== null
            ? `，当前 v${report.current_version}`
            : '，当前无生效版本'}
          {report.retire ? '，这是一次撤销' : ''}
        </span>
      </p>
      {report.problems.length ? (
        <ul className="policy-problems" aria-label="校验问题">
          {report.problems.map((problem, index) => (
            <li key={`${problem.path}-${index}`}>
              <code>{problem.path}</code> {problem.message}
            </li>
          ))}
        </ul>
      ) : null}
      {report.expansions.length ? (
        <p className="policy-expansion-heading" role="alert">
          {report.expansions.length} 项权限扩大，发布前逐条确认：
        </p>
      ) : (
        <p className="policy-no-expansion">没有权限扩大项。</p>
      )}
      <ExpansionList items={report.expansions} kind="expansion" />
      <ExpansionList items={report.narrowings} kind="narrowing" />
      <details className="policy-diff">
        <summary>
          差异（{report.diff.format}）：＋{report.diff.added} －
          {report.diff.removed} ～{report.diff.changed}
          {report.diff.truncated ? '，已截断' : ''}
        </summary>
        <ul>
          {report.diff.entries.map((entry, index) => (
            <li key={`${entry.path}-${index}`} data-op={entry.op}>
              <span className="policy-diff-op">{OP_LABEL[entry.op]}</span>
              <code>{entry.path}</code>
              {entry.before !== null ? (
                <span className="policy-diff-before">{entry.before}</span>
              ) : null}
              {entry.after !== null ? (
                <span className="policy-diff-after">{entry.after}</span>
              ) : null}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}
