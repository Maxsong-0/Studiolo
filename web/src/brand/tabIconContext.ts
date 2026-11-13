import { createContext, useContext, useEffect } from 'react';
import type { MarkTone } from './markTone';
import type { TabIcon } from './tabIcon';

/** main.tsx 启动时建好唯一的 TabIcon 放进来；没放（单测里单独渲染外壳）就不碰标签页图标。 */
export const TabIconContext = createContext<TabIcon | null>(null);

/**
 * 外壳把画法报给标签页图标。外壳卸载（比如会话失效退回登录页）时回到 mono：
 * 没有外壳就没人知道有没有事等你，不知道就不亮红。
 */
export function useTabIconTone(tone: MarkTone): void {
  const icon = useContext(TabIconContext);
  useEffect(() => {
    icon?.set(tone);
    return () => icon?.set('mono');
  }, [icon, tone]);
}
