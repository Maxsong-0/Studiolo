import { useEffect, useLayoutEffect, useRef, useState } from 'react';

function otherDialogOpen() {
  return Boolean(
    document.querySelector('dialog[open]:not([data-quick-switcher])'),
  );
}

export function useQuickSwitcher(enabled: boolean) {
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
        'dialog[open][data-quick-switcher]',
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
