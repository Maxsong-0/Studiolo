import type { View } from '../types';
import './shell.css';

/** 顶栏六个导航，顺序固定（设计规范第 4 节）。 */
const navigation: readonly { view: View; label: string }[] = [
  { view: 'cockpit', label: '驾驶舱' },
  { view: 'projects', label: '项目' },
  { view: 'documents', label: '文档' },
  { view: 'collab', label: '协作' },
  { view: 'rules', label: '规则' },
  { view: 'policy', label: '策略' },
];

/**
 * 主导航。宽屏在顶栏里，窄屏（<760px）由 shell.css 固定到底部成六格——同一份 DOM，
 * 不另渲染一套移动导航，免得同名按钮成双、读屏与键盘各走一遍。
 */
export function TopNav({
  view,
  onNavigate,
  counts,
}: {
  view: View;
  onNavigate: (view: View) => void;
  /** 视图旁的计数；null 表示当前快照给不出，显示「—」而不是假装是 0；没列出的视图不显示计数。 */
  counts: Partial<Record<View, number | null>>;
}) {
  return (
    <nav className="shell-nav" aria-label="主导航">
      {navigation.map((item) => {
        const count = counts[item.view];
        return (
          <button
            key={item.view}
            type="button"
            className="shell-nav-item"
            data-view={item.view}
            aria-current={view === item.view ? 'page' : undefined}
            aria-label={item.label}
            onClick={() => onNavigate(item.view)}
          >
            <span className="shell-nav-label">{item.label}</span>
            {count !== undefined ? (
              <span className="shell-nav-count" aria-hidden="true">
                {count ?? '—'}
              </span>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}
