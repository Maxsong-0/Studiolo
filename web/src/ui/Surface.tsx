import {
  createElement,
  type DialogHTMLAttributes,
  type HTMLAttributes,
  type Ref,
} from 'react';
import { cx } from './dom';
import './ui.css';

export type SurfaceElement =
  'div' | 'section' | 'aside' | 'article' | 'dialog' | 'form' | 'nav';

interface SurfaceOwnProps {
  /** 出现时 120ms 透明度＋4px 位移（减少动态时只留透明度）；自己用动效库驱动时关掉。 */
  appear?: boolean;
}

/** as='dialog'：收对话框自己的属性（onCancel、onClose、open …），ref 指向 HTMLDialogElement。 */
export type DialogSurfaceProps = SurfaceOwnProps &
  DialogHTMLAttributes<HTMLDialogElement> & {
    as: 'dialog';
    ref?: Ref<HTMLDialogElement>;
  };

/** 其余元素：通用 HTML 属性。 */
export type BlockSurfaceProps = SurfaceOwnProps &
  HTMLAttributes<HTMLElement> & {
    as?: Exclude<SurfaceElement, 'dialog'>;
    ref?: Ref<HTMLElement>;
  };

/** 属性类型按 as 区分：只有 dialog 收对话框属性，渲染上两者没有区别。 */
export type SurfaceProps = DialogSurfaceProps | BlockSurfaceProps;

/** 浮层框：纸色底、2px 墨线、10px 硬偏移阴影（规范第 1、8 节）。 */
export function Surface({
  as = 'div',
  appear = true,
  className,
  children,
  ...rest
}: SurfaceProps) {
  return createElement(
    as,
    {
      ...rest,
      className: cx('ui-surface', appear && 'ui-surface--appear', className),
    },
    children,
  );
}
