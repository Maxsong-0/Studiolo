import {
  createElement,
  type HTMLAttributes,
  type ReactNode,
  type Ref,
} from 'react';
import { cssVars, cx } from './dom';
import './ui.css';

export type GridFrameProps = HTMLAttributes<HTMLDivElement> & {
  /** 列数，缺省 12。窄屏（<760px）一律收成单列。 */
  columns?: number;
  /** grid-template-rows 原样传入，如 '300px minmax(0, 1fr) 190px'。 */
  rows?: string;
  ref?: Ref<HTMLDivElement>;
};

/** 1px 黑线分区的网格容器：容器墨色底＋1px 间隙，格子纸色底（规范第 3 节）。 */
export function GridFrame({
  columns = 12,
  rows,
  className,
  style,
  children,
  ...rest
}: GridFrameProps) {
  const count = Number.isFinite(columns)
    ? Math.max(1, Math.floor(columns))
    : 12;
  return (
    <div
      {...rest}
      className={cx('ui-grid', className)}
      style={{
        ...style,
        ...cssVars({
          '--ui-grid-columns': String(count),
          '--ui-grid-rows': rows,
        }),
      }}
    >
      {children}
    </div>
  );
}

export type CellElement =
  'section' | 'div' | 'article' | 'aside' | 'header' | 'footer' | 'nav' | 'li';

export type CellProps = HTMLAttributes<HTMLElement> & {
  /** 跨几列；缺省占满整行（12 列网格下即 12）。 */
  span?: number;
  /** 跨几行；缺省 1。 */
  rowSpan?: number;
  /** grid＝网格纸底，只给拓扑、泳道等画图区域用。 */
  surface?: 'plain' | 'grid';
  as?: CellElement;
  /** 去掉格子内边距，给标题栏这类自带网格的内容用。 */
  flush?: boolean;
  ref?: Ref<HTMLElement>;
};

const span = (value: number | undefined) =>
  value !== undefined && Number.isFinite(value) && value >= 1
    ? `span ${Math.floor(value)}`
    : undefined;

export function Cell({
  span: columnSpan,
  rowSpan,
  surface = 'plain',
  as = 'section',
  flush = false,
  className,
  style,
  children,
  ...rest
}: CellProps) {
  return createElement(
    as,
    {
      ...rest,
      className: cx(
        'ui-cell',
        surface === 'grid' && 'ui-cell--grid',
        flush && 'ui-cell--flush',
        className,
      ),
      style: {
        ...style,
        ...cssVars({
          '--ui-cell-column': span(columnSpan),
          '--ui-cell-row': span(rowSpan),
        }),
      },
    },
    children,
  );
}

export interface CellHeaderProps {
  title: ReactNode;
  /** 右侧等宽次要信息。 */
  meta?: ReactNode;
  /** 右侧动作（文字链接或按钮）。 */
  action?: ReactNode;
  /** 标题的标题级别，缺省 h2。 */
  level?: 2 | 3 | 4;
  className?: string;
}

/** 格子标题行：左 18px/800 标题，右等宽次要信息或动作。 */
export function CellHeader({
  title,
  meta,
  action,
  level = 2,
  className,
}: CellHeaderProps) {
  const Heading = `h${level}` as 'h2' | 'h3' | 'h4';
  const hasAside =
    (meta !== undefined && meta !== null) ||
    (action !== undefined && action !== null);
  return (
    <div className={cx('ui-cell-header', className)}>
      <Heading className="ui-cell-header__title">{title}</Heading>
      {hasAside && (
        <div className="ui-cell-header__aside">
          {meta !== undefined && meta !== null && (
            <span className="ui-cell-header__meta">{meta}</span>
          )}
          {action}
        </div>
      )}
    </div>
  );
}
