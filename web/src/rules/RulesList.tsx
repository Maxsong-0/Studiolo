import { useId } from 'react';
import type { PolicyStatus } from '../policyModel';
import { shortHash } from '../policyModel';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  GridFrame,
  StatusMark,
} from '../ui';
import { displayTime, draftSummary } from './labels';
import type { RulesModel } from './rulesModel';
import '../styles/policy.css';
import '../styles/rules.css';

/**
 * 规则列表（默认面板）：一行一条规则——名、生效版本、生效时刻、读者（能力集名单里列了它的 AI 类型）、
 * 有没有未发布的规则草案。只有草案的新规则与只出现在读名单里的规则也列出，版本写「未发布」。
 * 窄屏按规则分组，每个事实带原列名（与项目明细同一做法，表格语义靠显式 role 保住）。
 */
export function RulesList({
  model,
  status,
  onOpen,
  onCreate,
}: {
  model: RulesModel;
  status: PolicyStatus;
  onOpen: (subject: string) => void;
  onCreate: () => void;
}) {
  const heading = useId();
  return (
    <GridFrame className="rules-grid">
      <Cell span={12} className="rules-cell" aria-labelledby={heading}>
        <CellHeader
          title={<span id={heading}>全部规则</span>}
          meta={
            <>
              {model.rules.length} 条 · 生效中摘要{' '}
              <code title={status.effective_hash}>
                {shortHash(status.effective_hash)}
              </code>
            </>
          }
          action={
            <Button variant="primary" size="sm" onClick={onCreate}>
              新建规则
            </Button>
          }
        />
        {model.unreadable.length ? (
          <p className="policy-form__error" role="alert">
            {model.unreadable.join('、')}{' '}
            的能力集读不出规则名单：「读者」一栏没有计入这些类型。
          </p>
        ) : null}
        {model.rules.length ? (
          <table className="policy-table rules-table" role="table">
            <caption className="sr-only">规则一览</caption>
            <thead>
              <tr role="row">
                <th scope="col" role="columnheader">
                  规则
                </th>
                <th scope="col" role="columnheader" className="is-number">
                  版本
                </th>
                <th scope="col" role="columnheader" className="is-number">
                  生效时刻
                </th>
                <th scope="col" role="columnheader">
                  读者
                </th>
                <th scope="col" role="columnheader">
                  未发布草案
                </th>
              </tr>
            </thead>
            <tbody>
              {model.rules.map((row) => {
                const drafts = draftSummary(row.drafts);
                return (
                  <tr key={row.subject} role="row">
                    <th scope="row" role="rowheader">
                      <button
                        type="button"
                        className="policy-queue__pick"
                        aria-label={`${row.subject}，编辑`}
                        onClick={() => onOpen(row.subject)}
                      >
                        {row.subject}
                      </button>
                    </th>
                    <td role="cell" data-label="版本" className="is-number">
                      {row.entry ? `v${row.entry.version}` : '未发布'}
                    </td>
                    <td role="cell" data-label="生效时刻" className="is-number">
                      {row.entry ? displayTime(row.entry.activated_at) : '—'}
                    </td>
                    <td
                      role="cell"
                      data-label="读者"
                      className="rules-table__readers"
                    >
                      {row.readers.length ? row.readers.join('、') : '无'}
                    </td>
                    <td role="cell" data-label="未发布草案">
                      {drafts ? (
                        <StatusMark state={drafts.state}>
                          {drafts.text}
                        </StatusMark>
                      ) : (
                        '—'
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        ) : (
          <EmptyState
            title="还没有规则"
            action={
              <Button size="sm" onClick={onCreate}>
                新建规则
              </Button>
            }
          >
            <p>
              规则的唯一母本是策略平面里的规则类条目：新建后先成为规则草案，校验并经二次认证发布才生效。
            </p>
          </EmptyState>
        )}
        <p className="policy-hint">
          读者＝能力集 read.rules_whitelist 里列了这条规则的 AI
          类型；缺这个字段的类型读不到任何规则。人这一侧永远能读全部规则。
        </p>
      </Cell>
    </GridFrame>
  );
}
