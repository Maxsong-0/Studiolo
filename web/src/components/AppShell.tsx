import type { ReactNode } from 'react';
import type { View } from '../types';
import { TopNav } from '../shell/TopNav';
import '../shell/shell.css';

/**
 * 外壳（T-130 设计规范第 5 节）：46px 顶栏，底边 3px 黑线。左边字标＋五个导航；
 * 右边状态胶囊、⌘K 入口与账户等工具。窄屏顶栏只留字标、胶囊与 ⌘K 图标，
 * 工具收到黑线下方一行，导航固定到底部。
 */
export function AppShell({
  view,
  onNavigate,
  actionCount,
  documentCount,
  status,
  command,
  tools,
  children,
}: {
  view: View;
  onNavigate: (view: View) => void;
  /** 待我动作条数，标在驾驶舱旁；null 表示还没有快照，显示「—」而不是假装是 0。 */
  actionCount: number | null;
  /** 文档总数；null 表示当前快照给不出这个计数，界面显示「—」而不是假装是 0。 */
  documentCount: number | null;
  /** 状态胶囊。 */
  status?: ReactNode;
  /** ⌘K 入口。 */
  command?: ReactNode;
  /** 账户、文件操作、内存草稿等工具入口。 */
  tools?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="shell">
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>
      <header className="shell-top">
        <div className="shell-brand">
          Studiolo
          <span className="shell-brand-dot" aria-hidden="true">
            .
          </span>
        </div>
        <TopNav
          view={view}
          onNavigate={onNavigate}
          counts={{ cockpit: actionCount, documents: documentCount }}
        />
        {status}
        {command}
        <div className="shell-tools">{tools}</div>
      </header>
      <main id="main-content" className="shell-main" tabIndex={-1}>
        {children}
      </main>
    </div>
  );
}
