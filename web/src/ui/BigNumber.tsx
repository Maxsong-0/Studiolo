import type { ReactNode } from 'react';
import { cx } from './dom';
import './ui.css';

export interface BigNumberProps {
  /** 读数；null 表示拿不到，显示「—」，绝不补一个数。 */
  value: number | null;
  /** 补零到几位，缺省 2（5 显示为 05）。 */
  digits?: number;
  label: ReactNode;
  note?: ReactNode;
  className?: string;
}

function padded(value: number, digits: number): string {
  const abs = Math.abs(value);
  const body = Number.isInteger(abs)
    ? String(abs).padStart(digits, '0')
    : String(abs);
  return value < 0 ? `-${body}` : body;
}

/** 巨号：每页只出现一次（规范第 3 节）；窄屏降为 72px。 */
export function BigNumber({
  value,
  digits = 2,
  label,
  note,
  className,
}: BigNumberProps) {
  const width = Number.isFinite(digits) ? Math.max(1, Math.floor(digits)) : 2;
  const shown =
    value !== null && Number.isFinite(value) ? padded(value, width) : '—';
  const spoken = value !== null && Number.isFinite(value) ? String(value) : '—';
  return (
    <div className={cx('ui-big-number', className)}>
      {shown === spoken ? (
        <span className="ui-big-number__value">{shown}</span>
      ) : (
        <span className="ui-big-number__value">
          {/* 补零只是排版：读屏读自然数 */}
          <span aria-hidden="true">{shown}</span>
          <span className="sr-only">{spoken}</span>
        </span>
      )}
      <span className="ui-big-number__label">{label}</span>
      {note !== undefined && note !== null && (
        <span className="ui-big-number__note">{note}</span>
      )}
    </div>
  );
}
