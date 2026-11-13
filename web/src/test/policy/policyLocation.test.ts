import { describe, expect, it } from 'vitest';
import {
  policyPanelSlot,
  policyTab,
  readingVersion,
  selectedDraft,
  wantsQueueFocus,
} from '../../policy/policyLocation';

const at = (panel: string | null, item: string | null = null) => ({
  panel,
  item,
});
const QUEUE = [{ id: 'A' }, { id: 'B' }, { id: 'C' }];

describe('策略页的位置换算（T-198）', () => {
  it('panel 定标签：缺省是现状，pending、submit 各是各的；不认识的值当现状，不猜', () => {
    expect(policyTab(at(null))).toBe('now');
    expect(policyTab(at('pending'))).toBe('pending');
    expect(policyTab(at('submit'))).toBe('submit');
    for (const panel of ['now', 'Pending', 'queue', 'versions', '待处理'])
      expect(policyTab(at(panel)), panel).toBe('now');
    // 现状里的 item 是版本号，不影响在哪个标签。
    expect(policyTab(at(null, '3'))).toBe('now');
  });

  it('写回 panel：现状记作 null，其余写标签名；读回来是同一个标签', () => {
    expect(policyPanelSlot('now')).toBeNull();
    expect(policyPanelSlot('pending')).toBe('pending');
    expect(policyPanelSlot('submit')).toBe('submit');
    for (const tab of ['now', 'pending', 'submit'] as const)
      expect(policyTab(at(policyPanelSlot(tab)))).toBe(tab);
  });

  it('旧写法 item=desired：没有 panel 时落在待处理、要把队列滚进视野并聚焦；带了 panel 就不是带路的记号', () => {
    expect(policyTab(at(null, 'desired'))).toBe('pending');
    expect(wantsQueueFocus(at(null, 'desired'))).toBe(true);
    expect(wantsQueueFocus(at('pending', 'desired'))).toBe(false);
    expect(wantsQueueFocus(at('pending', 'A'))).toBe(false);
    expect(wantsQueueFocus(at(null, '3'))).toBe(false);
    expect(wantsQueueFocus(at(null))).toBe(false);
    // 不认识的 panel 加上 desired：当现状，不当旧链接。
    expect(policyTab(at('queue', 'desired'))).toBe('now');
  });

  it('选中的草案：位置写了、而且还在队列里就是它；否则第一份；队列空就没有', () => {
    expect(selectedDraft(at('pending', 'B'), QUEUE)).toEqual({ id: 'B' });
    expect(selectedDraft(at('pending'), QUEUE)).toEqual({ id: 'A' });
    // 草案已经不在队列里（发布了、撤回了、别人处理了）：回到第一份，不停在一份看不到的东西上。
    expect(selectedDraft(at('pending', 'gone'), QUEUE)).toEqual({ id: 'A' });
    // 旧写法带路过来：`desired` 不是草案号。
    expect(selectedDraft(at(null, 'desired'), QUEUE)).toEqual({ id: 'A' });
    expect(selectedDraft(at('pending', 'B'), [])).toBeNull();
    // 别的标签里的 item 不是草案号：现状里的 `B` 是（不存在的）版本号。
    expect(selectedDraft(at(null, 'B'), QUEUE)).toEqual({ id: 'A' });
  });

  it('在读的版本：只有现状里的 item 才是版本号，原样交出去由面板对版本列表', () => {
    expect(readingVersion(at(null, '3'))).toBe('3');
    expect(readingVersion(at(null))).toBeNull();
    expect(readingVersion(at('queue', '3'))).toBe('3');
    expect(readingVersion(at('pending', '3'))).toBeNull();
    expect(readingVersion(at('submit', '3'))).toBeNull();
    expect(readingVersion(at(null, 'desired'))).toBeNull();
  });
});
