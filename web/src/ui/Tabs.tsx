import type { KeyboardEvent } from 'react';
import { cx } from './dom';
import './ui.css';

export interface TabItem {
  id: string;
  label: string;
  count?: number | null;
}

export interface TabsProps {
  items: readonly TabItem[];
  value: string;
  onChange: (id: string) => void;
  ariaLabel: string;
  /**
   * 给了就把标签与面板连起来：标签 id 为 `${idPrefix}-tab-${id}`，
   * 并以 aria-controls 指向 `${idPrefix}-panel-${id}`（面板由调用方渲染）。
   */
  idPrefix?: string;
  className?: string;
}

/**
 * 子标签（role=tablist）：当前项红色 3px 下划线。
 * 左右方向键循环切换、Home/End 到首尾，焦点随之移动；只有当前项在 Tab 序列里。
 */
export function Tabs({
  items,
  value,
  onChange,
  ariaLabel,
  idPrefix,
  className,
}: TabsProps) {
  const selectedIndex = items.findIndex((item) => item.id === value);
  const focusIndex = selectedIndex === -1 ? 0 : selectedIndex;

  const select = (index: number, from: HTMLElement) => {
    const item = items[index];
    if (!item) return;
    const tabs = from
      .closest('[role="tablist"]')
      ?.querySelectorAll<HTMLElement>('[role="tab"]');
    tabs?.[index]?.focus();
    if (item.id !== value) onChange(item.id);
  };

  const onKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    const last = items.length - 1;
    let next: number;
    switch (event.key) {
      case 'ArrowRight':
        next = index >= last ? 0 : index + 1;
        break;
      case 'ArrowLeft':
        next = index <= 0 ? last : index - 1;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = last;
        break;
      default:
        return;
    }
    event.preventDefault();
    select(next, event.currentTarget);
  };

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className={cx('ui-tabs', className)}
    >
      {items.map((item, index) => {
        const selected = item.id === value;
        return (
          <button
            key={item.id}
            type="button"
            role="tab"
            id={idPrefix ? `${idPrefix}-tab-${item.id}` : undefined}
            aria-controls={
              idPrefix ? `${idPrefix}-panel-${item.id}` : undefined
            }
            aria-selected={selected}
            tabIndex={index === focusIndex ? 0 : -1}
            className="ui-tab"
            onClick={() => {
              if (!selected) onChange(item.id);
            }}
            onKeyDown={(event) => onKeyDown(event, index)}
          >
            {/* data-text 让 CSS 预留加粗后的宽度，切换时标签不左右跳 */}
            <span className="ui-tab__label" data-text={item.label}>
              {item.label}
            </span>
            {/* 计数前的空格让可访问名字读成「任务与回执 12」；在 flex 里不占位 */}
            {item.count !== undefined && item.count !== null && (
              <>
                {' '}
                <span className="ui-tab__count">{item.count}</span>
              </>
            )}
          </button>
        );
      })}
    </div>
  );
}
