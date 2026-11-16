import { createContext } from 'react';

/** 顶栏「更多」菜单交给顶栏里别的部件用的入口（T-199）。 */
export interface ToolsMenuHandle {
  /**
   * 展开菜单，并把焦点交给菜单里匹配这个选择器的那个入口（没有匹配的就只展开）。
   * 状态浮层的「前往账户与会话」用它：账户入口平时收在菜单里，不展开就接不住焦点。
   */
  reveal(selector: string): void;
}

/** 外面没有「更多」菜单（单独渲染某个部件、测试）时是 null：入口就在原地，直接找它。 */
export const ToolsMenuContext = createContext<ToolsMenuHandle | null>(null);
