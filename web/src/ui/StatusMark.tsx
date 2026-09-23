import type { ReactNode } from 'react';
import { cx } from './dom';
import './ui.css';

/**
 * ok/active＝墨色实心方块（正常、运行）；pending＝墨色空心方块（进行中、待定）；
 * alert＝红色实心方块（故障、等你）；dormant＝灰色虚线框（休眠）。
 */
export type StatusState = 'ok' | 'active' | 'pending' | 'alert' | 'dormant';

export interface StatusMarkProps {
  state: StatusState;
  /** 状态文字必须给：颜色与方块只是辅助，不单独表意。 */
  children: ReactNode;
  className?: string;
}

export function StatusMark({ state, children, className }: StatusMarkProps) {
  return (
    <span className={cx('ui-status', className)} data-state={state}>
      <span className="ui-status__mark" aria-hidden="true" />
      <span className="ui-status__text">{children}</span>
    </span>
  );
}
