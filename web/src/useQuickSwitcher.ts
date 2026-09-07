import { useEffect, useState } from 'react';

function otherDialogOpen() {
  return Boolean(
    document.querySelector('dialog[open]:not([data-quick-switcher])'),
  );
}

export function useQuickSwitcher(enabled: boolean) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if (
        !enabled ||
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
  }, [enabled]);
  return {
    open: open && enabled,
    show: () => {
      if (enabled && !otherDialogOpen()) setOpen(true);
    },
    close: () => setOpen(false),
  };
}
