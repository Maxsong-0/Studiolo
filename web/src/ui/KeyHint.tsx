import { cx } from './dom';
import './ui.css';

export interface KeyHintProps {
  /** 依次按下的键，如 ['⌘', 'K']。 */
  keys: readonly string[];
  className?: string;
}

/** 等宽键帽文字；嵌套 kbd 表示组合键。 */
export function KeyHint({ keys, className }: KeyHintProps) {
  return (
    <kbd className={cx('ui-keyhint', className)}>
      {keys.map((key, index) => (
        <kbd key={`${index}-${key}`}>{key}</kbd>
      ))}
    </kbd>
  );
}
