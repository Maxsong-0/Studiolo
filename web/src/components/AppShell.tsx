import type { ReactNode } from 'react';
import type { View } from '../types';
import { BrandMark } from '../brand/BrandMark';
import { markToneFor } from '../brand/markTone';
import { useTabIconTone } from '../brand/tabIconContext';
import { TopNav } from '../shell/TopNav';
import '../shell/shell.css';

/**
 * 外壳（T-130 设计规范第 5 节）：46px 顶栏，底边 3px 黑线。左边品牌格（标志＋「工作台」）、当前库名与六个导航；
 * 右边状态胶囊、⌘K 入口与账户等工具。窄屏顶栏只留品牌格、库名、胶囊与 ⌘K 图标，
 * 工具收到黑线下方一行，导航固定到底部。
 */
export function AppShell({
  view,
  onNavigate,
  sourceLabel,
  actionCount,
  documentCount,
  status,
  command,
  tools,
  children,
}: {
  view: View;
  onNavigate: (view: View) => void;
  /**
   * 当前库名（工作台快照的 source_label），任何宽度都显示在品牌格右侧（承接 T-117）；
   * null 或不传表示还没读到快照，显示「—」并写明原因，不拿别的字顶替。
   */
  sourceLabel?: string | null;
  /**
   * 待我动作条数，标在驾驶舱旁；null 表示还没有快照，显示「—」而不是假装是 0。
   * 标志的影子和标签页图标也看这个数：确知大于 0 才红（规范第 14 节）。
   */
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
  // 顶栏标志、标签页图标与「驾驶舱 N」角标看同一个数，三处不会各说各的。
  const tone = markToneFor(actionCount);
  useTabIconTone(tone);
  return (
    <div className="shell">
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>
      <header className="shell-top">
        <div className="shell-brand">
          <BrandMark tone={tone} size={16} label="Studiolo" />
          工作台
        </div>
        <LibraryName label={sourceLabel ?? null} />
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

/**
 * 当前库名：等宽 11.5px，超长由样式省略，title 给全名；读屏读到的是「当前库」加全名，
 * 省略号只是视觉上的截断。读不到库名时显示「—」，title 与读屏都写明原因。
 */
function LibraryName({ label }: { label: string | null }) {
  if (label !== null && label.trim())
    return (
      <span className="shell-library" title={label}>
        <span className="sr-only">当前库：</span>
        <span className="shell-library-name">{label}</span>
      </span>
    );
  const reason =
    label === null ? '尚未读到工作台快照' : '工作台快照没有给出库名';
  return (
    <span className="shell-library" title={reason}>
      <span className="sr-only">当前库：{reason}</span>
      <span className="shell-library-name" aria-hidden="true">
        —
      </span>
    </span>
  );
}
