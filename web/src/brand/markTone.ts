/**
 * 标志的两种画法（设计规范第 14 节）：standard＝影子是信号红，mono＝影子与纸同色。
 * 红色在全站只表示「有事等你」，所以会随状态变的地方（标签页图标、顶栏）只在确知有事时才用 standard；
 * 不随状态变的地方（登录页）固定用 standard。
 */
export type MarkTone = 'standard' | 'mono';

/**
 * 待我动作条数 → 画法。条数就是 cockpit/pending.ts 的 pendingCount：与顶栏角标、驾驶舱巨号同一口径。
 * null 是「还没读到」或「读不出」：不知道就不亮红，和「没事」一样画成 mono。
 */
export function markToneFor(pending: number | null): MarkTone {
  return pending !== null && pending > 0 ? 'standard' : 'mono';
}
