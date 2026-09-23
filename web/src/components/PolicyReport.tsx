import type { DiffEntry, Expansion, Report } from '../policyModel';
import { shortHash } from '../policyModel';
import { Term } from '../policy/Term';
import { CHANGE_TERMS } from '../policy/terms';
import { StatusMark } from '../ui';
import { cx } from '../ui/dom';
import '../styles/policy.css';

/** 接口的报告只分扩大与收窄，不单列未知项：计数显示「—」并说明原因，绝不补一个数。 */
const UNKNOWN_REASON =
  '校验报告只区分扩大项与收窄项，接口没有单列未知项；没归入两类的改动见下方差异。';

/**
 * 权限改动逐条列表：待确认的扩大项行首红色实心方块＋红字「扩大」，收窄项空心方块＋中性色「收窄」。
 * 「扩大」「收窄」都以文字出现，颜色只是辅助。
 * `historical`：已发布版本里记下的扩大项是确认过的历史，既不等人也不是故障（规范第 1 节红色只给这两种），
 * 与收窄项一样用中性色，「扩大」二字照旧写出。
 */
export function ChangeList({
  items,
  kind,
  historical = false,
  label,
}: {
  items: Expansion[];
  kind: 'expansion' | 'narrowing';
  historical?: boolean;
  /** 列表的可访问名；缺省「权限扩大项」「权限收窄项」。 */
  label?: string;
}) {
  if (!items.length) return null;
  const expansion = kind === 'expansion';
  const signal = expansion && !historical;
  return (
    <ul
      className={cx(
        'policy-changes',
        signal
          ? 'policy-changes--expansion'
          : expansion
            ? 'policy-changes--history'
            : 'policy-changes--narrowing',
      )}
      aria-label={label ?? (expansion ? '权限扩大项' : '权限收窄项')}
    >
      {items.map((item, index) => (
        <li key={`${item.path}-${index}`}>
          <StatusMark
            className="policy-changes__tag"
            state={signal ? 'alert' : 'pending'}
          >
            {expansion
              ? CHANGE_TERMS.expansion.human
              : CHANGE_TERMS.narrowing.human}
          </StatusMark>
          <span className="policy-changes__body">
            <code className="policy-changes__path">{item.path}</code>
            {item.before !== null ? <span> 原：{item.before}</span> : null}
            {item.after !== null ? <span> 改为：{item.after}</span> : null}
            <em className="policy-changes__note">{item.note}</em>
          </span>
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

const OP_NAME: Record<DiffEntry['op'], string> = {
  added: '新增',
  removed: '删除',
  changed: '改动',
};

/**
 * 校验报告：结论、改动方向计数、问题、扩大项（红）、收窄项（中性）、差异。
 * 不渲染 Markdown，正文一律当文本；差异只是改动清单，不分红绿。
 */
export function PolicyReport({ report }: { report: Report }) {
  const expansions = report.expansions.length;
  return (
    <div
      className={cx('policy-report', report.ok ? 'is-ok' : 'is-rejected')}
      role="group"
      aria-label="校验报告"
    >
      <div className="policy-report__head">
        <h4 className="policy-report__title">校验报告</h4>
        <StatusMark
          className="policy-report__verdict"
          state={report.ok ? 'ok' : 'alert'}
        >
          {report.ok ? '校验通过' : '校验未通过'}
        </StatusMark>
        <p className="policy-report__counts">
          <span className={cx(expansions > 0 && 'is-signal')}>
            {expansions > 0 ? (
              <span className="ui-signal-mark" aria-hidden="true" />
            ) : null}
            {CHANGE_TERMS.expansion.human} {expansions}
          </span>
          <span aria-hidden="true"> · </span>
          <span>
            {CHANGE_TERMS.narrowing.human} {report.narrowings.length}
          </span>
          <span aria-hidden="true"> · </span>
          <span title={UNKNOWN_REASON}>
            {CHANGE_TERMS.unknown.human} —
            <span className="sr-only">（{UNKNOWN_REASON}）</span>
          </span>
        </p>
      </div>
      <p className="policy-report__base">
        基于生效中摘要{' '}
        <code title={report.base_effective_hash}>
          {shortHash(report.base_effective_hash)}
        </code>
        {report.current_version !== null
          ? `，当前 v${report.current_version}`
          : '，当前无生效版本'}
        {report.retire ? '，这是一次撤销' : ''}
      </p>
      {report.problems.length ? (
        <ul className="policy-problems" aria-label="校验问题">
          {report.problems.map((problem, index) => (
            <li key={`${problem.path}-${index}`}>
              <StatusMark className="policy-problems__tag" state="alert">
                问题
              </StatusMark>
              <span>
                <code>{problem.path}</code> {problem.message}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {expansions ? (
        <p className="policy-report__alert" role="alert">
          {expansions} 项权限扩大，发布前逐条确认：
        </p>
      ) : (
        <p className="policy-report__none">没有权限扩大项。</p>
      )}
      <ChangeList items={report.expansions} kind="expansion" />
      <ChangeList items={report.narrowings} kind="narrowing" />
      <details className="policy-diff">
        <summary>
          <Term name="diff" />（{report.diff.format}）：＋{report.diff.added} －
          {report.diff.removed} ～{report.diff.changed}
          {report.diff.truncated ? '，已截断' : ''}
        </summary>
        {report.diff.entries.length ? (
          <ul>
            {report.diff.entries.map((entry, index) => (
              <li key={`${entry.path}-${index}`} data-op={entry.op}>
                <span className="policy-diff__op" title={OP_NAME[entry.op]}>
                  {OP_LABEL[entry.op]}
                  <span className="sr-only">{OP_NAME[entry.op]}</span>
                </span>
                <code>{entry.path}</code>
                {entry.before !== null ? (
                  <span className="policy-diff__before">{entry.before}</span>
                ) : null}
                {entry.after !== null ? (
                  <span className="policy-diff__after">{entry.after}</span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="policy-hint">没有逐项差异。</p>
        )}
      </details>
    </div>
  );
}
