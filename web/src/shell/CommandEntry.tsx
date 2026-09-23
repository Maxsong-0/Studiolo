import { Icon } from '../components/Icon';
import './shell.css';

/**
 * ⌘K 入口：打开现有的快速切换。可访问名保留「快速切换」，与快捷键、弹窗标题一致；
 * 窄屏只剩图标（shell.css）。快捷键监听仍在 useQuickSwitcher 里，不挂在这里的状态上（BUG-011）。
 */
export function CommandEntry({
  onOpen,
  disabled,
}: {
  onOpen: () => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      className="shell-command"
      onClick={onOpen}
      disabled={disabled}
      aria-keyshortcuts="Meta+K Control+K"
    >
      <Icon name="search" />
      <span className="sr-only">快速切换：</span>
      <kbd>⌘K</kbd>
      <span className="shell-command-hint">跳转、搜索或执行动作</span>
    </button>
  );
}
