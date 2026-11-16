import { useId, useSyncExternalStore } from 'react';
import { themePreference } from './preference';
import type { ThemeChoice, ThemePreference } from './preference';
import './theme.css';

const CHOICES: readonly { value: ThemeChoice; label: string }[] = [
  { value: 'system', label: '跟随系统' },
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
];

/**
 * 深浅色三选一（T-199；设计规范第 5 节）：跟随系统、浅色、深色，选了立刻生效。
 * 三个按钮排成一条，选中的那个是黑底块（`aria-pressed`），和顶栏当前导航项同一种画法；
 * 每个都能用 Tab 走到，不另做方向键。选择只记在这台浏览器里（见 preference.ts）。
 */
export function ThemeSwitch({
  preference = themePreference(),
}: {
  /** 缺省用全站那一份；测试里换成自己的。 */
  preference?: ThemePreference;
}) {
  const choice = useSyncExternalStore(preference.subscribe, preference.choice);
  const labelId = useId();
  return (
    <div className="theme-switch" role="group" aria-labelledby={labelId}>
      <span id={labelId} className="theme-switch__label">
        外观
      </span>
      <div className="theme-switch__choices">
        {CHOICES.map((item) => (
          <button
            key={item.value}
            type="button"
            className="theme-switch__choice"
            aria-pressed={choice === item.value}
            onClick={() => preference.set(item.value)}
          >
            {item.label}
          </button>
        ))}
      </div>
    </div>
  );
}
