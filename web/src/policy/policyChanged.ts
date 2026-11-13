import { createContext } from 'react';

/**
 * 「策略平面刚被写过」的通知（T-198）：提交、校验、撤回、发布、回滚、撤销接入做完之后调一次。
 * 全站那一份草案读取（顶栏「驾驶舱 N」角标用的那份，App 持有）据此安静重读，角标不必等人回到驾驶舱。
 * 由 usePolicyActions 统一发出，策略页、规则页都经过它；缺省什么都不做（单独渲染某个视图时没有 App）。
 */
export const PolicyChangedContext = createContext<() => void>(() => {});
