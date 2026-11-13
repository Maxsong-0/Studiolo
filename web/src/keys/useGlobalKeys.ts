import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { View } from '../types';
import { singleKeysPreference, type SingleKeysPreference } from './preference';
import { createKeySequence, decideKey } from './shortcuts';

/** 快捷键一览的 dialog 带这个属性：它开着时「?」还能把它关掉，别的单键一律不响应。 */
export const KEYBOARD_SHEET_ATTRIBUTE = 'data-keyboard-sheet';

export interface KeyBinding {
  run(): void;
  /** 按住不放时要不要连着触发：移动选中行要，别的只认第一下。 */
  repeat?: boolean;
  /** 焦点落在 `target` 上时这个键算不算数；不给就是都算。不算数时按键原样放行。 */
  when?(target: EventTarget | null): boolean;
}

/** 键名（单个字符用小写，功能键用 `KeyboardEvent.key` 的名字）到做法。 */
export type KeyBindings = Readonly<Record<string, KeyBinding>>;

export interface KeyScope {
  /** 这一处有没有接上全站监听；没接上（单独渲染的视图）时键位提示不显示。 */
  readonly live: boolean;
  /** 单键快捷键的开关：监听、一览、键位提示看的是同一个。 */
  readonly preference: SingleKeysPreference;
  /** 登记一组键位，返回注销函数；每次按键时现取，登记的一方不用为了换做法重新登记。 */
  register(bindings: () => KeyBindings): () => void;
  /** 打开快捷键一览。 */
  showSheet(): void;
}

const NO_KEYS: KeyScope = {
  live: false,
  preference: {
    subscribe: () => () => {},
    enabled: () => false,
    set: () => {},
    sync: () => {},
  },
  register: () => () => {},
  showSheet: () => {},
};

/** 全站那一个快捷键监听；不在 App 里（单独渲染的视图、测试）时是什么都不做的那一个。 */
export const KeyScopeContext = createContext<KeyScope>(NO_KEYS);

/** 后登记的排前面：同一个键两处都登记时，以后挂上来的（更靠里的那一处）为准。 */
function findBinding(
  sources: readonly (() => KeyBindings)[],
  key: string,
): KeyBinding | null {
  for (let index = sources.length - 1; index >= 0; index -= 1) {
    const bindings = sources[index]();
    if (Object.hasOwn(bindings, key)) return bindings[key];
  }
  return null;
}

/**
 * 给 App 用：全站唯一的单键快捷键监听（T-197；判断规则在 shortcuts.ts 的 decideKey）。
 *
 * BUG-011 的修法照搬：监听只挂一次，要用到的回调放进 ref、在布局阶段刷新——
 * 提交和被动效果是两个调度任务，监听要是跟着依赖重挂，这段间隙里的按键会按上一轮的旧值处理。
 * 和 ⌘K 的监听互不相干：这里一律不理带 Ctrl、⌘、Alt 的键。
 */
export function useGlobalKeys({
  onGo,
  onShowSheet,
  preference = singleKeysPreference(),
}: {
  /** G 再按字母：去那个视图。 */
  onGo: (view: View) => void;
  onShowSheet: () => void;
  /** 单键快捷键的开关；缺省用全站那一个，测试里换成自己的。 */
  preference?: SingleKeysPreference;
}): KeyScope {
  const handlers = useRef({ onGo, onShowSheet });
  useLayoutEffect(() => {
    handlers.current = { onGo, onShowSheet };
  });
  const [sources] = useState<(() => KeyBindings)[]>(() => []);
  const [scope] = useState<KeyScope>(() => ({
    live: true,
    preference,
    register(bindings) {
      sources.push(bindings);
      return () => {
        const index = sources.indexOf(bindings);
        if (index >= 0) sources.splice(index, 1);
      };
    },
    showSheet: () => handlers.current.onShowSheet(),
  }));

  useEffect(() => {
    const sequence = createKeySequence(() => Date.now());
    function keydown(event: KeyboardEvent) {
      const dialogs = document.querySelectorAll('dialog[open]');
      const sheet =
        dialogs.length === 1 &&
        dialogs[0].hasAttribute(KEYBOARD_SHEET_ATTRIBUTE)
          ? dialogs[0]
          : null;
      const outcome = decideKey(event, {
        sequence,
        singleKeys: preference.enabled(),
        dialogOpen: dialogs.length > 0,
        sheetOpen: sheet !== null,
        bound: (key, repeat, target) => {
          const binding = findBinding(sources, key);
          return (
            binding !== null &&
            (!repeat || binding.repeat === true) &&
            (binding.when?.(target) ?? true)
          );
        },
      });
      if (outcome.kind === 'none') return;
      // 归快捷键管的键都拦下：免得同一下又触发浏览器自己的「边打边找」。
      event.preventDefault();
      if (outcome.kind === 'go') handlers.current.onGo(outcome.view);
      else if (outcome.kind === 'bound')
        findBinding(sources, outcome.key)?.run();
      else if (outcome.kind === 'sheet') {
        // 一览开着就关：和 ⌘K 一样走 dialog 的 cancel，由它自己做可中断的收起并归还焦点。
        if (sheet)
          sheet.dispatchEvent(new Event('cancel', { cancelable: true }));
        else handlers.current.onShowSheet();
      }
    }
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [preference, sources]);

  return scope;
}

/**
 * 登记一组键位（驾驶舱队列的 J／K／Enter／H）。`bindings` 每次重画都可以是新对象：
 * 最新的那份放进 ref，按键时现取；只有 `active` 变了才重新登记。
 */
export function useKeyBindings(bindings: KeyBindings, active = true): void {
  const scope = useContext(KeyScopeContext);
  const latest = useRef(bindings);
  useLayoutEffect(() => {
    latest.current = bindings;
  });
  useLayoutEffect(() => {
    if (!active) return;
    return scope.register(() => latest.current);
  }, [scope, active]);
}
