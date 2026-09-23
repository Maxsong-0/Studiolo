import type { KeyboardEvent } from 'react';
import '../styles/workspace.css';

/**
 * 视图内的子标签（WAI-ARIA tabs，手动激活）：方向键只移动焦点，回车或点击才切换——
 * 每个面板都会读接口，不该一路按方向键一路发请求。B 轮会换成通用 ui/Tabs。
 */
export function ViewTabs<T extends string>({
  label,
  tabs,
  selected,
  onSelect,
  idPrefix,
}: {
  label: string;
  tabs: readonly { id: T; label: string }[];
  selected: T;
  onSelect: (id: T) => void;
  /** tab 与 tabpanel 的 id 前缀：tab 为 `${idPrefix}-tab-${id}`，面板为 `${idPrefix}-panel`。 */
  idPrefix: string;
}) {
  function move(event: KeyboardEvent<HTMLButtonElement>) {
    const siblings = [
      ...(event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
        '[role="tab"]',
      ) ?? []),
    ];
    const index = siblings.indexOf(event.currentTarget);
    const target =
      event.key === 'ArrowRight'
        ? siblings[(index + 1) % siblings.length]
        : event.key === 'ArrowLeft'
          ? siblings[(index - 1 + siblings.length) % siblings.length]
          : event.key === 'Home'
            ? siblings[0]
            : event.key === 'End'
              ? siblings[siblings.length - 1]
              : null;
    if (!target) return;
    event.preventDefault();
    target.focus();
  }
  return (
    <div className="view-tabs" role="tablist" aria-label={label}>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          id={`${idPrefix}-tab-${tab.id}`}
          aria-selected={tab.id === selected}
          aria-controls={`${idPrefix}-panel`}
          tabIndex={tab.id === selected ? 0 : -1}
          onClick={() => onSelect(tab.id)}
          onKeyDown={move}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
