import { cx } from './dom';
import './ui.css';

export interface CircledNumberProps {
  n: number | string;
  /** 红色实心反白：能在拓扑上定位的待办；否则黑色空心。 */
  filled?: boolean;
  /** 给读屏的完整说法（如「待办 1，已在拓扑上标出」）；不给时读圈里的数字。 */
  label?: string;
  className?: string;
}

/** 圈号：全站唯一的圆形。 */
export function CircledNumber({
  n,
  filled = false,
  label,
  className,
}: CircledNumberProps) {
  return (
    <span
      className={cx('ui-circled', filled && 'ui-circled--filled', className)}
      data-filled={filled ? 'true' : 'false'}
      role={label ? 'img' : undefined}
      aria-label={label || undefined}
    >
      {n}
    </span>
  );
}
