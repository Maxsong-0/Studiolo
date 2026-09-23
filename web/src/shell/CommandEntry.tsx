import { Icon } from '../components/Icon';
import './shell.css';

/**
 * ⌘K 入口：打开动作面板（设计规范第 5、8 节）。250px 等宽描边框「⌘K 跳转、搜索或执行动作」；
 * 可访问名以「动作面板」开头，与面板标题、快捷键一致；窄屏只剩图标（shell.css）。
 * 键位沿用 shell.css 管的普通 kbd：禁用时整块随按钮一起变灰（KeyHint 自带墨色，会在禁用态里留下一块黑字）。
 * 快捷键监听在 commands/useCommandPalette 里，不挂在这里的状态上（BUG-011）。
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
      aria-haspopup="dialog"
      aria-keyshortcuts="Meta+K Control+K"
    >
      <Icon name="search" />
      <span className="sr-only">动作面板：</span>
      <kbd>⌘K</kbd>
      <span className="shell-command-hint">跳转、搜索或执行动作</span>
    </button>
  );
}
