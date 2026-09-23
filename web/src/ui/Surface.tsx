import { createElement, type HTMLAttributes, type Ref } from 'react';
import { cx } from './dom';
import './ui.css';

export type SurfaceElement =
  'div' | 'section' | 'aside' | 'article' | 'dialog' | 'form' | 'nav';

export type SurfaceProps = HTMLAttributes<HTMLElement> & {
  as?: SurfaceElement;
  /** 出现时 120ms 透明度＋4px 位移（减少动态时只留透明度）；自己用动效库驱动时关掉。 */
  appear?: boolean;
  ref?: Ref<HTMLElement>;
};

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
