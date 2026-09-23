import type { ReactNode } from 'react';
import { Button } from './Button';
import { cx } from './dom';
import './ui.css';

export interface EmptyStateProps {
  title: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  className?: string;
}

/** 空态：朴素文字＋可选动作。 */
export function EmptyState({
  title,
  children,
  action,
  className,
}: EmptyStateProps) {
  return (
    <div className={cx('ui-state', 'ui-state--empty', className)}>
      <p className="ui-state__title">{title}</p>
      {children !== undefined && children !== null && (
        <div className="ui-state__body">{children}</div>
      )}
      {action !== undefined && action !== null && (
        <div className="ui-state__action">{action}</div>
      )}
    </div>
  );
}

export interface ErrorStateProps {
  title: ReactNode;
  children?: ReactNode;
  onRetry?: () => void;
  /** 重试按钮的名字，缺省「重试」；同页有多处可重试时写具体，如「重试目录」。 */
  retryLabel?: string;
  className?: string;
}

/** 错误态（role=alert）：红色方块＋文字，可带重试。 */
export function ErrorState({
  title,
  children,
  onRetry,
  retryLabel = '重试',
  className,
}: ErrorStateProps) {
  return (
    <div role="alert" className={cx('ui-state', 'ui-state--error', className)}>
      <p className="ui-state__title">{title}</p>
      {children !== undefined && children !== null && (
        <div className="ui-state__body">{children}</div>
      )}
      {onRetry && (
        <div className="ui-state__action">
          <Button size="sm" onClick={onRetry}>
            {retryLabel}
          </Button>
        </div>
      )}
    </div>
  );
}

export interface SkeletonProps {
  /** 占位条数，缺省 3。 */
  lines?: number;
  /** 读屏与肉眼都能看到的读取说明，缺省「正在读取…」。 */
  label?: string;
  className?: string;
}

/** 读取中占位：斜线纹条不进无障碍树，外层 role=status 报告读取中。 */
export function Skeleton({
  lines = 3,
  label = '正在读取…',
  className,
}: SkeletonProps) {
  const count = Number.isFinite(lines) ? Math.max(0, Math.floor(lines)) : 3;
  return (
    <div role="status" className={cx('ui-skeleton', className)}>
      <span className="ui-skeleton__label">{label}</span>
      <span className="ui-skeleton__lines" aria-hidden="true">
        {Array.from({ length: count }, (_, index) => (
          <span key={index} className="ui-skeleton__line" />
        ))}
      </span>
    </div>
  );
}
