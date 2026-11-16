import { useId, useState } from 'react';
import type { Report } from '../policyModel';
import { Button, StatusMark } from '../ui';
import {
  serverDirection,
  type CapabilityCell,
  type CapabilityComparison,
  type ChangeDirection,
  type JsonScalar,
  type UnreadableReason,
} from './capabilityDiff';
import { capabilityField } from './capabilityLabels';
import { CHANGE_TERMS } from './terms';
import '../styles/policy-tabs.css';

/** 服务端校验报告里判定扩大、收窄的两张表。 */
type Verdicts = Pick<Report, 'expansions' | 'narrowings'>;

/** 读不出结构时的一句话：正在写的时候说怎么改，审草案的时候指给校验报告。 */
function unreadable(
  side: 'before' | 'after',
  reason: UnreadableReason,
  typing: boolean,
): string {
  if (side === 'before')
    return typing
      ? '生效中那一版的正文读不出结构，这里列不出差别；提交后看校验报告。'
      : '生效中那一版的正文读不出结构，这张表列不出来；看下面的校验报告。';
  if (!typing)
    return '这份草案的正文读不出结构（不是一个 JSON 对象），这张表列不出来；看下面的校验报告。';
  if (reason === 'empty')
    return '正文还是空的：写上 JSON 之后，这里会列出它和现在的差别。';
  if (reason === 'not_object')
    return '正文不是一个 JSON 对象（要用花括号包起来）：改好之后，这里会列出差别。';
  return '正文还不是一段完整的 JSON：写完整之后，这里会列出差别。';
}

function scalarText(value: JsonScalar): string {
  if (value === true) return '是';
  if (value === false) return '否';
  return value === null ? 'null' : String(value);
}

/** 服务端判的方向：扩大用红色实心方块（等你确认），收窄用中性色；两个字都写出来，颜色只是辅助。 */
function Direction({ direction }: { direction: ChangeDirection }) {
  return (
    <StatusMark
      className="policy-capdiff__direction"
      state={direction === 'expansion' ? 'alert' : 'pending'}
    >
      {CHANGE_TERMS[direction].human}
    </StatusMark>
  );
}

function Value({
  cell,
  side,
  path,
  changed,
  verdicts,
}: {
  cell: CapabilityCell;
  side: 'before' | 'after';
  path: string;
  changed: boolean;
  verdicts: Verdicts | null;
}) {
  if (cell.kind === 'absent')
    return (
      <span className="policy-capdiff__none">
        <span aria-hidden="true">—</span>
        <span className="sr-only">没有</span>
      </span>
    );
  if (cell.kind === 'scalar') {
    // 单值字段的方向只标在「改后」那一格。
    const direction =
      verdicts && changed && side === 'after'
        ? serverDirection(verdicts, path, side, null)
        : null;
    return (
      <span className="policy-capdiff__scalar">
        {typeof cell.value === 'string' ? (
          <code>{cell.value}</code>
        ) : (
          scalarText(cell.value)
        )}
        {direction ? <Direction direction={direction} /> : null}
      </span>
    );
  }
  if (!cell.items.length)
    return <span className="policy-capdiff__none">空</span>;
  return (
    <ul className="policy-capdiff__list">
      {cell.items.map((item) => {
        const direction =
          verdicts && item.onlyHere
            ? serverDirection(verdicts, path, side, item.text)
            : null;
        return (
          <li key={item.text} data-only={item.onlyHere ? side : undefined}>
            <code>{item.text}</code>
            {item.onlyHere ? (
              <span className="policy-capdiff__tag">
                {side === 'before' ? '去掉' : '新增'}
              </span>
            ) : null}
            {direction ? <Direction direction={direction} /> : null}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * 能力差异表（T-198）：能力集的每个字段一行，列「现在」「改后」。名单逐项标「新增」「去掉」；
 * 有改动的行写「有改动」并加重。不变的行先收着（审的人先看改了什么，校验报告和发布按钮不被挤到一屏之外），
 * 汇总那一行写明有几项不变，点「列出不变的 N 项」按字段原顺序全部列出、颜色放淡。
 * 给了校验报告（`verdicts`）就照它在对应的项上标「扩大」「收窄」；没给（正在写、还没提交）就不标，
 * 这张表自己不判方向。表格语义靠显式的 role 保住：窄屏时行会改排成一组一组。
 */
export function CapabilityDiff({
  comparison,
  before,
  after,
  verdicts = null,
  typing = false,
}: {
  comparison: CapabilityComparison;
  /** 「现在」那一列比的是什么，如「生效中 v14」「没有生效版本」。 */
  before: string;
  /** 「改后」那一列是什么，如「这份草案」「撤销之后」「正在写的」。 */
  after: string;
  verdicts?: Verdicts | null;
  /** 正文正在写：读不出结构时说怎么改，而不是指给校验报告。 */
  typing?: boolean;
}) {
  const tableId = useId();
  const [showAll, setShowAll] = useState(false);
  if (comparison.kind === 'unreadable')
    return (
      <p className="policy-hint">
        {unreadable(comparison.side, comparison.reason, typing)}
      </p>
    );
  const { rows, changed } = comparison;
  if (!rows.length)
    return <p className="policy-hint">两边都没有内容，没有可比的。</p>;
  const unchanged = rows.length - changed;
  const listed = showAll ? rows : rows.filter((row) => row.changed);
  return (
    <div className="policy-capdiff">
      <p className="policy-capdiff__summary">
        {changed
          ? `${changed} 项有改动，${unchanged} 项不变`
          : `${rows.length} 项都和现在一样`}
        {unchanged ? (
          <Button
            variant="text"
            size="sm"
            aria-expanded={showAll}
            aria-controls={listed.length ? tableId : undefined}
            onClick={() => setShowAll((open) => !open)}
          >
            {showAll ? '收起' : '列出'}不变的 {unchanged} 项
          </Button>
        ) : null}
      </p>
      {/* 收着的时候只有有改动的行；一行都没有（两边一样、又没展开）就不出表。 */}
      {listed.length ? (
        <table className="policy-capdiff__table" role="table" id={tableId}>
          <caption className="sr-only">能力集的每一项：现在和改后</caption>
          <thead role="rowgroup">
            <tr role="row">
              <th scope="col" role="columnheader">
                能力
              </th>
              <th scope="col" role="columnheader">
                现在<span className="policy-capdiff__which">{before}</span>
              </th>
              <th scope="col" role="columnheader">
                改后<span className="policy-capdiff__which">{after}</span>
              </th>
            </tr>
          </thead>
          <tbody role="rowgroup">
            {listed.map((row) => {
              const field = capabilityField(row.path);
              return (
                <tr
                  key={row.path}
                  role="row"
                  data-changed={row.changed ? 'true' : undefined}
                >
                  <th scope="row" role="rowheader">
                    <span className="policy-capdiff__name">
                      {field?.label ?? row.path}
                    </span>
                    {row.changed ? (
                      <span className="policy-capdiff__flag">有改动</span>
                    ) : null}
                    <span className="policy-capdiff__about">
                      {field ? (
                        <>
                          <code lang="en">{row.path}</code> · {field.note}
                        </>
                      ) : (
                        '不认识的字段，照原样列出'
                      )}
                    </span>
                  </th>
                  <td role="cell" data-label={`现在（${before}）`}>
                    <Value
                      cell={row.before}
                      side="before"
                      path={row.path}
                      changed={row.changed}
                      verdicts={verdicts}
                    />
                  </td>
                  <td role="cell" data-label={`改后（${after}）`}>
                    <Value
                      cell={row.after}
                      side="after"
                      path={row.path}
                      changed={row.changed}
                      verdicts={verdicts}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : null}
    </div>
  );
}
