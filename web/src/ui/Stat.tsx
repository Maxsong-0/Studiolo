import type { ReactNode } from 'react';
import { cx } from './dom';
import './ui.css';

export type StatTone = 'default' | 'signal' | 'muted';

export interface StatProps {
  label: ReactNode;
  /** 读数；null 显示「—」并用 reason 说明原因，绝不编造。 */
  value: ReactNode;
  unit?: ReactNode;
  /** signal 只用于「等你」或「出问题」，同时画红色方块，不单靠颜色表意。 */
  tone?: StatTone;
  /** 读数缺失或标红的原因：进 title，并给读屏一份。 */
  reason?: string;
  children?: ReactNode;
  className?: string;
}

/** 等宽标签＋大号读数（72px）。每页至多一个大号读数区。 */
export function Stat({
  label,
  value,
  unit,
  tone = 'default',
  reason,
  children,
  className,
}: StatProps) {
  const missing = value === null || value === undefined;
  return (
    <div
      className={cx('ui-stat', `ui-stat--${tone}`, className)}
      title={reason}
    >
      <span className="ui-stat__label">
        {tone === 'signal' && (
          <span className="ui-signal-mark" aria-hidden="true" />
        )}
        {label}
      </span>
      <span className="ui-stat__value">
        {missing ? '—' : value}
        {!missing && unit !== undefined && unit !== null && (
          <span className="ui-stat__unit">{unit}</span>
        )}
      </span>
      {reason && <span className="sr-only">{reason}</span>}
      {children}
    </div>
  );
}
