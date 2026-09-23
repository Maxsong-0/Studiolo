import { useEffect, useLayoutEffect, useRef, useState } from 'react';

/** 动作面板的 dialog 带这个属性；「别的模态」就是任何不带它的已打开 dialog。 */
export const COMMAND_PALETTE_ATTRIBUTE = 'data-command-palette';

function otherDialogOpen() {
  return Boolean(
    document.querySelector(`dialog[open]:not([${COMMAND_PALETTE_ATTRIBUTE}])`),
  );
}

/**
 * ⌘K／Ctrl+K 打开或收起动作面板；不在别的模态上叠开。
 * BUG-011 的修法原样保留：快捷键监听只挂一次，可用状态放进 ref，并在布局阶段刷新。
 */
export function useCommandPalette(enabled: boolean) {
  const [open, setOpen] = useState(false);
  const available = useRef(enabled);
  // 快捷键按「已提交」的可用状态判定：提交与被动效果是两个调度任务，
  // 若把监听挂在 [enabled] 上，这段间隙里的按键会按上一轮的旧值被挡掉。
  useLayoutEffect(() => {
    available.current = enabled;
  }, [enabled]);
  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if (
        !available.current ||
        event.isComposing ||
        event.altKey ||
        !(event.metaKey || event.ctrlKey) ||
        event.key.toLowerCase() !== 'k'
      )
        return;
      event.preventDefault();
      if (otherDialogOpen()) return;
      const dialog = document.querySelector<HTMLDialogElement>(
        `dialog[open][${COMMAND_PALETTE_ATTRIBUTE}]`,
      );
      if (dialog)
        dialog.dispatchEvent(new Event('cancel', { cancelable: true }));
      else setOpen(true);
    }
    window.addEventListener('keydown', shortcut);
    return () => window.removeEventListener('keydown', shortcut);
  }, []);
  return {
    open: open && enabled,
    show: () => {
      if (enabled && !otherDialogOpen()) setOpen(true);
    },
    close: () => setOpen(false),
  };
}
