/**
 * 键盘快捷键（T-197）：全站只有这一张表，「?」一览照它写，监听照它做，写的和做的不会两样。
 * 这里全是纯函数和常量，不碰 React 与 DOM 事件的挂载；时钟由调用方给。
 *
 * 规矩：
 * - 单键快捷键只在「不会被当成打字」的时候生效：焦点不在输入框、文本框、下拉框、可编辑区；
 *   没有弹层开着（一览自己除外，再按一次「?」关掉它）；没按着 Ctrl、⌘、Alt；输入法不在组字。
 * - G 之后 {@link GO_TIMEOUT_MS} 内的下一个键归这次跳转：是表里的字母就跳，不是就作废。
 *   所以「G H」里的 H 不会落到驾驶舱的「稍后」上，「G R」里的 R 也不会落到「退回」上。
 * - 会写东西的键（驾驶舱的 A 批准、R 退回）只打开确认那一步，不直接写（T-200）。
 * - Enter 落在按钮、链接、表单控件上时是它们自己的，不抢。
 * - 带 Shift 的字母不算（只有「?」本来就要按 Shift）。
 * - 别处已经处理掉的键（`defaultPrevented`）不再处理；按住不放只认第一下，
 *   除非登记时说了可以连着触发（J／K 连着走）。
 */
import type { View } from '../types';

/**
 * G 之后按哪个字母去哪个视图。按英文首字母取（Home、Projects、Documents、Collab、Rules）；
 * 策略的 P 已经给了项目，取 Strategy 的 S。不按拼音：拼音首字母有重复（项目、协作都是 X）。
 */
export const GO_TARGETS: readonly {
  key: string;
  view: View;
  label: string;
}[] = [
  { key: 'h', view: 'cockpit', label: '驾驶舱' },
  { key: 'p', view: 'projects', label: '项目' },
  { key: 'd', view: 'documents', label: '文档' },
  { key: 'c', view: 'collab', label: '协作' },
  { key: 'r', view: 'rules', label: '规则' },
  { key: 's', view: 'policy', label: '策略' },
];

export const GO_KEY = 'g';
/** 按下 G 之后等下一个键等多久。 */
export const GO_TIMEOUT_MS = 1500;
export const SHEET_KEY = '?';

/** 驾驶舱队列登记的键位（见 cockpit/QueueCell.tsx）。 */
export const QUEUE_KEYS = {
  next: 'j',
  previous: 'k',
  open: 'Enter',
  later: 'h',
  /** 批准并发布选中的策略草案：只打开执行预览（T-200）。 */
  approve: 'a',
  /** 退回（撤回）选中的策略草案：只打开确认框（T-200）。 */
  reject: 'r',
} as const;

/** 一种按法：`keys` 是要按的键；`sequence` 为真表示一个接一个按，否则是同时按的组合键。 */
export interface KeyWay {
  keys: readonly string[];
  sequence?: boolean;
}

export interface ShortcutRow {
  /** 并列的几种按法，哪种都行。 */
  ways: readonly KeyWay[];
  does: string;
}

export interface ShortcutGroup {
  title: string;
  /** 这一组在哪儿生效。 */
  where: string;
  /** 这一组是不是单键快捷键（关掉开关后失效）。 */
  single: boolean;
  rows: readonly ShortcutRow[];
}

/** 「?」一览的全部内容。 */
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = [
  {
    title: '动作面板',
    where: '任何视图',
    single: false,
    rows: [
      {
        ways: [{ keys: ['⌘', 'K'] }, { keys: ['Ctrl', 'K'] }],
        does: '打开或收起动作面板：跳转、搜索、执行动作',
      },
      { ways: [{ keys: ['Esc'] }], does: '关闭弹层' },
    ],
  },
  {
    title: '跳转',
    where: '任何视图',
    single: true,
    rows: [
      ...GO_TARGETS.map((target) => ({
        ways: [{ keys: ['G', target.key.toUpperCase()], sequence: true }],
        does: `去${target.label}`,
      })),
      { ways: [{ keys: [SHEET_KEY] }], does: '打开或关闭这张一览' },
    ],
  },
  {
    title: '等你处理',
    where: '驾驶舱',
    single: true,
    rows: [
      { ways: [{ keys: ['J'] }], does: '选下一件' },
      { ways: [{ keys: ['K'] }], does: '选上一件' },
      { ways: [{ keys: ['Enter'] }], does: '去处理选中的那一件' },
      {
        ways: [{ keys: ['H'] }],
        does: '稍后：挪到队尾，4 小时后或它有新进展时回到原位；再按一次放回',
      },
      {
        ways: [{ keys: ['A'] }],
        does: '批准并发布选中的策略草案：只展开执行预览，点了确认才发布；再按一次收起',
      },
      {
        ways: [{ keys: ['R'] }],
        does: '退回选中的策略草案（自己交的是撤回）：只展开确认框，点了确认才撤回；再按一次收起',
      },
    ],
  },
];

/** 判断用到的那几样；真实的 KeyboardEvent 直接满足。 */
export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  /** 按住不放时系统自动重复出来的那些。 */
  repeat: boolean;
  defaultPrevented: boolean;
  target: EventTarget | null;
}

const TYPING =
  'textarea, select, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"], [role="textbox"], [role="combobox"], [role="searchbox"], [role="spinbutton"]';

/** 不接受打字的 input 类型：焦点在它们上面时字母键不算打字。 */
const NON_TEXT_INPUTS = new Set([
  'button',
  'checkbox',
  'color',
  'file',
  'image',
  'radio',
  'range',
  'reset',
  'submit',
]);

/** 这个目标里能不能打字：能打字的地方，字母键是它的。 */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest(TYPING)) return true;
  const input = target.closest('input');
  return input !== null && !NON_TEXT_INPUTS.has(input.type);
}

const OWNS_ENTER =
  'button, a[href], input, select, textarea, summary, [role="button"], [role="link"], [role="tab"], [role="option"], [role="menuitem"], [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]';

/** 这个目标自己会处理 Enter（按钮、链接、表单控件）：Enter 留给它。 */
export function ownsEnter(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(OWNS_ENTER) !== null;
}

/** 单个字符的键不分大小写（开着大写锁定也认）；Enter、Escape 这类名字原样。 */
export function normalizeKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

/** G 之后等下一个键的那一小段状态。 */
export interface KeySequence {
  /** 现在是不是在等 G 后面的那个键（过了时限就不算了）。 */
  pending(): boolean;
  /** 按下了 G。 */
  arm(): void;
  /** 在等的时候来了一个键：是跳转字母就给出视图，不是给 null；两种情况都不再等。 */
  resolve(key: string): View | null;
  cancel(): void;
}

export function createKeySequence(
  now: () => number,
  timeoutMs = GO_TIMEOUT_MS,
): KeySequence {
  let armedAt: number | null = null;
  return {
    pending: () => armedAt !== null && now() - armedAt <= timeoutMs,
    arm() {
      armedAt = now();
    },
    resolve(key) {
      armedAt = null;
      return GO_TARGETS.find((target) => target.key === key)?.view ?? null;
    },
    cancel() {
      armedAt = null;
    },
  };
}

/** 一次按键该怎么处理。 */
export type KeyOutcome =
  /** 不归快捷键管，原样放行。 */
  | { kind: 'none' }
  /** 按下了 G，等下一个键。 */
  | { kind: 'pending' }
  /** G 之后按了表里没有的键：这一键作废，不落到别处。 */
  | { kind: 'cancelled' }
  | { kind: 'go'; view: View }
  /** 打开或关闭一览。 */
  | { kind: 'sheet' }
  /** 交给登记了这个键的地方。 */
  | { kind: 'bound'; key: string };

const NONE: KeyOutcome = { kind: 'none' };

export interface KeyState {
  sequence: KeySequence;
  /** 单键快捷键开着没有（「?」一览里的开关）。 */
  singleKeys: boolean;
  /** 有弹层开着。 */
  dialogOpen: boolean;
  /** 开着的弹层就是一览自己。 */
  sheetOpen: boolean;
  /**
   * 这个键现在有没有地方登记，而且焦点落在 `target` 上时它愿意接；
   * `repeat` 为真时只问「登记时说了可以连着触发」的。
   */
  bound(key: string, repeat: boolean, target: EventTarget | null): boolean;
}

/**
 * 决定一次按键归谁。除了 G 序列的那一小段状态，没有别的副作用：真正去跳转、开一览、调登记的函数，
 * 由调用方照返回值做。
 */
export function decideKey(event: KeyLike, state: KeyState): KeyOutcome {
  const { sequence } = state;
  if (
    event.defaultPrevented ||
    event.isComposing ||
    event.ctrlKey ||
    event.metaKey ||
    event.altKey
  )
    return NONE;
  if (!state.singleKeys || isTypingTarget(event.target)) {
    sequence.cancel();
    return NONE;
  }
  const key = normalizeKey(event.key);
  if (state.dialogOpen) {
    sequence.cancel();
    return state.sheetOpen && key === SHEET_KEY && !event.repeat
      ? { kind: 'sheet' }
      : NONE;
  }
  // 单独按下修饰键（为了打「?」先按住 Shift）不算 G 后面的那个键。
  if (key === 'Shift' || key === 'CapsLock') return NONE;
  if (event.repeat)
    return !event.shiftKey &&
      !sequence.pending() &&
      state.bound(key, true, event.target)
      ? { kind: 'bound', key }
      : NONE;
  if (sequence.pending()) {
    // 只有字符键归这次跳转；Enter、Tab、方向键这些让序列作废，自己照常处理。
    if (key.length === 1) {
      if (key === GO_KEY && !event.shiftKey) {
        sequence.arm();
        return { kind: 'pending' };
      }
      const view = event.shiftKey ? null : sequence.resolve(key);
      sequence.cancel();
      return view ? { kind: 'go', view } : { kind: 'cancelled' };
    }
    sequence.cancel();
  }
  if (key === SHEET_KEY) return { kind: 'sheet' };
  if (event.shiftKey) return NONE;
  if (key === GO_KEY) {
    sequence.arm();
    return { kind: 'pending' };
  }
  if (key === 'Enter' && ownsEnter(event.target)) return NONE;
  return state.bound(key, false, event.target) ? { kind: 'bound', key } : NONE;
}
