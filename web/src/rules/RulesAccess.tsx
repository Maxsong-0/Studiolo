import { useId, useState } from 'react';
import type { PolicyStatus } from '../policyModel';
import { Cell, CellHeader, EmptyState, GridFrame } from '../ui';
import { DraftPanel } from './DraftPanel';
import { RulesFeedback } from './RulesFeedback';
import { accessChange } from './accessChange';
import { accessCell } from './rulesModel';
import type { RulesModel, TypeColumn } from './rulesModel';
import type { PolicyActions } from './usePolicyActions';
import '../styles/policy.css';
import '../styles/rules.css';

const ORIGIN = 'access';

/**
 * 列头下的说明：缺字段即零规则（D4）、名单读不出（出问题，红字）、有待发布改动、类型已撤销接入。
 */
function columnNotes(column: TypeColumn): { text: string; signal: boolean }[] {
  const notes: { text: string; signal: boolean }[] = [];
  if (column.whitelist.kind === 'unset')
    notes.push({ text: '未设（零规则）', signal: false });
  if (column.whitelist.kind === 'invalid')
    notes.push({ text: '名单读不出', signal: true });
  if (column.ownerDraft) notes.push({ text: '有待发布改动', signal: false });
  if (column.revoked) notes.push({ text: '已撤销接入', signal: false });
  return notes;
}

/**
 * 读权限面板：矩阵行＝规则、列＝AI 类型（能力集 subject）。改一格＝为该类型提交一份能力集草案，
 * 只改 read.rules_whitelist、其余字段与键顺序原样保留；校验报告与发布（执行预览＋二次认证）在下方一格。
 * 一列已有你提交、还没发布的能力集草案时按草案显示；再改格子就在这份草案上追加（交新撤旧），
 * 同一轮发布可以改好几格；改回生效原状则只撤回草案。只有名单读不出时才不让改（不猜结构）。
 * 窄屏同一份表格改排成按规则分组的开关列表。
 */
export function RulesAccess({
  model,
  status,
  actions,
}: {
  model: RulesModel;
  status: PolicyStatus;
  actions: PolicyActions;
}) {
  const ids = useId();
  const [problem, setProblem] = useState<string | null>(null);
  const pending = model.types.filter((column) => column.ownerDraft !== null);

  function toggle(column: TypeColumn, rule: string, allowed: boolean) {
    setProblem(null);
    let change: ReturnType<typeof accessChange>;
    try {
      change = accessChange(column, rule, allowed);
    } catch (cause) {
      setProblem(cause instanceof Error ? cause.message : '不能改这一格。');
      return;
    }
    if (change.kind === 'withdraw') {
      actions.withdraw(ORIGIN, change.draft);
      return;
    }
    const input = {
      kind: 'capability_set' as const,
      subject: column.type,
      body: change.body,
    };
    if (change.replaces) actions.amend(ORIGIN, input, change.replaces);
    else actions.submit(ORIGIN, input);
  }

  return (
    <GridFrame className="rules-grid">
      <Cell span={12} className="rules-cell" aria-labelledby={`${ids}-matrix`}>
        <CellHeader
          title={<span id={`${ids}-matrix`}>哪类 AI 能读哪条规则</span>}
          meta={`${model.rules.length} 条规则 × ${model.types.length} 类 AI`}
        />
        <RulesFeedback
          origin={ORIGIN}
          notice={actions.notice}
          error={actions.error}
        />
        {problem ? (
          <p className="policy-form__error" role="alert">
            {problem}
          </p>
        ) : null}
        {status.global_revoked ? (
          <p className="policy-form__error" role="alert">
            全局撤销生效中：所有 AI 都被拒绝，读名单暂时不起作用。
          </p>
        ) : null}
        {!model.types.length ? (
          <EmptyState title="还没有生效的能力集">
            <p>
              没有能力集的 AI
              类型一律零权限，读不到任何规则。先在「策略」里发布该类型的能力集。
            </p>
          </EmptyState>
        ) : !model.rules.length ? (
          <EmptyState title="还没有规则">
            <p>先在「编辑」里新建并发布规则，再来设谁能读。</p>
          </EmptyState>
        ) : (
          <div
            className="rules-matrix-scroll"
            role="region"
            aria-label="读权限矩阵"
            tabIndex={0}
          >
            <table className="rules-matrix" role="table">
              <caption className="sr-only">
                读权限矩阵：行是规则，列是 AI 类型
              </caption>
              <thead>
                <tr role="row">
                  <th scope="col" role="columnheader">
                    规则
                  </th>
                  {model.types.map((column) => {
                    const notes = columnNotes(column);
                    return (
                      <th key={column.type} scope="col" role="columnheader">
                        <span className="rules-matrix__type">
                          {column.type}
                        </span>
                        <span className="rules-matrix__note">
                          能力集 v{column.entry.version}
                          {notes.map((note) => (
                            <span
                              key={note.text}
                              className={note.signal ? 'is-signal' : undefined}
                            >
                              {note.text}
                            </span>
                          ))}
                        </span>
                      </th>
                    );
                  })}
                </tr>
              </thead>
              <tbody>
                {model.rules.map((row, rowIndex) => (
                  <tr key={row.subject} role="row">
                    <th
                      scope="row"
                      role="rowheader"
                      className="rules-matrix__rule"
                    >
                      {row.subject}
                      {row.entry ? null : (
                        <span className="rules-matrix__note">未发布</span>
                      )}
                    </th>
                    {model.types.map((column, columnIndex) => {
                      const cell = accessCell(column, row.subject);
                      const describe = `${ids}-cell-${rowIndex}-${columnIndex}`;
                      const state =
                        cell.shown === null
                          ? '读不出'
                          : cell.shown
                            ? '可读'
                            : '不可读';
                      return (
                        <td key={column.type} role="cell">
                          <button
                            type="button"
                            role="switch"
                            className="rules-switch"
                            aria-checked={cell.shown === true}
                            aria-describedby={
                              cell.pending ? describe : undefined
                            }
                            data-pending={cell.pending ? 'true' : undefined}
                            disabled={actions.busy || cell.locked}
                            title={
                              cell.locked
                                ? `${column.type} 的能力集名单读不出，不能在这里改；请到「策略」页核对正文。`
                                : column.ownerDraft
                                  ? `${column.type} 有一份待发布草案：改这一格会追加到它上面。`
                                  : undefined
                            }
                            onClick={() =>
                              // 按当前显示的状态取反：有待发布草案时显示的是草案，不是生效版本。
                              toggle(column, row.subject, cell.shown !== true)
                            }
                          >
                            <span className="sr-only">
                              {column.type} 读取 {row.subject}
                            </span>
                            <span
                              className="rules-switch__type"
                              aria-hidden="true"
                            >
                              {column.type}
                            </span>
                            <span
                              className="rules-switch__mark"
                              aria-hidden="true"
                            />
                            <span
                              className="rules-switch__state"
                              aria-hidden="true"
                            >
                              {state}
                              {cell.pending ? '（待发布）' : ''}
                            </span>
                          </button>
                          {cell.pending ? (
                            <span id={describe} hidden>
                              待发布：生效中{cell.effective ? '可读' : '不可读'}
                              ，发布后才改
                            </span>
                          ) : null}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p className="policy-hint">
          改一格＝为该类型提交一份能力集草案，只改
          read.rules_whitelist；校验后经二次认证发布才生效。缺这个字段的类型读不到任何规则。
        </p>
      </Cell>
      {pending.length ? (
        <Cell
          span={12}
          className="rules-cell"
          aria-labelledby={`${ids}-pending`}
        >
          <CellHeader
            title={<span id={`${ids}-pending`}>待发布的读权限改动</span>}
            meta={`${pending.length} 类`}
          />
          {pending.map((column) =>
            column.ownerDraft ? (
              <DraftPanel
                key={column.type}
                record={column.ownerDraft}
                title={`能力集 ${column.type}`}
                label={`能力集 ${column.type}`}
                origin={ORIGIN}
                status={status}
                actions={actions}
                others={column.others}
              />
            ) : null,
          )}
        </Cell>
      ) : null}
    </GridFrame>
  );
}
