import type { ReactNode } from 'react';
import { cssVars, cx } from './dom';
import './ui.css';

export interface TitleBlockCell {
  label: ReactNode;
  /** 读数；null 显示「—」，原因写进格子的 title，绝不编造。 */
  value: ReactNode | null;
  reason?: string;
  /** signal 只用于「等你」或「出问题」，同时画红色方块，不单靠颜色表意。 */
  tone?: 'default' | 'signal';
}

export interface TitleBlockProps {
  /** 图名，如「驾驶舱」。 */
  title: ReactNode;
  /** 图名下的等宽小字，如日期与副本代次；字符串里的换行照原样换行。 */
  subtitle?: ReactNode;
  cells: readonly TitleBlockCell[];
  className?: string;
}

/** 读数格固定排三行；列数随格数增加。 */
const ROWS = 3;

/**
 * 图纸标题栏（规范第 3 节）：第一列跨三行放图名，其余格子「等宽小标签＋15px 等宽值」，
 * 按行从左到右排；最后一行不满时用空白格补齐，免得露出底下的墨色。窄屏改成单列清单。
 */
export function TitleBlock({
  title,
  subtitle,
  cells,
  className,
}: TitleBlockProps) {
  const columns = Math.ceil(cells.length / ROWS);
  const fillers = columns * ROWS - cells.length;
  return (
    <div
      className={cx('ui-title-block', className)}
      style={cssVars({ '--ui-title-columns': String(Math.max(1, columns)) })}
    >
      <div className="ui-title-block__name">
        <span className="ui-title-block__kicker">图名</span>
        <p className="ui-title-block__title">{title}</p>
        {subtitle !== undefined && subtitle !== null && (
          <span className="ui-title-block__subtitle">{subtitle}</span>
        )}
      </div>
      {cells.length > 0 && (
        <ul className="ui-title-block__cells" role="list">
          {cells.map((cell, index) => {
            const missing = cell.value === null || cell.value === undefined;
            const signal = cell.tone === 'signal';
            return (
              <li
                key={index}
                className={cx(
                  'ui-title-block__cell',
                  signal && 'ui-title-block__cell--signal',
                )}
                title={cell.reason}
              >
                <span className="ui-title-block__label">{cell.label}</span>
                <span className="ui-title-block__value">
                  {signal && (
                    <span className="ui-signal-mark" aria-hidden="true" />
                  )}
                  {missing ? '—' : cell.value}
                </span>
                {cell.reason && <span className="sr-only">{cell.reason}</span>}
              </li>
            );
          })}
          {Array.from({ length: fillers }, (_, index) => (
            <li
              key={`filler-${index}`}
              className="ui-title-block__filler"
              aria-hidden="true"
            />
          ))}
        </ul>
      )}
    </div>
  );
}
