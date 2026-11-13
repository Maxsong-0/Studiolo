/**
 * 策略页的位置换算（T-198）：三个子标签和各自选中的那一项，都从位置的 `panel`、`item` 两格算出来。
 * 纯函数：不读 URL、不碰 React。写位置由面板自己做，这里只定「写成什么」。
 *
 * - `panel`：现状是缺省面板，记作 null；待处理 `pending`；提交 `submit`；不认识的值当现状，不猜。
 * - `item`：现状里是在读的版本号，待处理里是选中的草案号，提交里不用。
 * - 旧链接（拆标签之前）：没有 `panel`、`item=desired` 表示「直达待处理队列」，⌘K 的两条命令至今写的也是它。
 *   版本正文、草案正文绝不进位置，这里只有标签名、版本号、草案号。
 */
import { POLICY_QUEUE_ITEM } from '../viewLocation';
import type { ViewLocation } from '../viewLocation';

export type PolicyTab = 'now' | 'pending' | 'submit';

type Slots = Pick<ViewLocation, 'panel' | 'item'>;

/** 标签在位置 `panel` 格里的写法。 */
export function policyPanelSlot(tab: PolicyTab): string | null {
  return tab === 'now' ? null : tab;
}

/** 位置是不是旧写法的「直达待处理队列」：落地要把队列滚进视野并聚焦。 */
export function wantsQueueFocus({ panel, item }: Slots): boolean {
  return panel === null && item === POLICY_QUEUE_ITEM;
}

/** 在看哪个标签。 */
export function policyTab(location: Slots): PolicyTab {
  if (location.panel === 'pending' || location.panel === 'submit')
    return location.panel;
  return wantsQueueFocus(location) ? 'pending' : 'now';
}

/**
 * 待处理里选中哪一份：位置写了、而且那一份还在队列里，就是它；否则第一份；队列空就没有。
 * 只是把细节展开给人看，不做任何动作。
 */
export function selectedDraft<T extends { id: string }>(
  location: Slots,
  actionable: readonly T[],
): T | null {
  if (policyTab(location) === 'pending' && location.item !== null) {
    const wanted = actionable.find((record) => record.id === location.item);
    if (wanted) return wanted;
  }
  return actionable[0] ?? null;
}

/** 现状里在读哪一版的正文：位置里的版本号（原样的字符串）；别的标签没有。 */
export function readingVersion(location: Slots): string | null {
  return policyTab(location) === 'now' ? location.item : null;
}
