import type { ReactNode } from 'react';
import type { OpenSource, Project, View } from '../types';
import { Icon } from './Icon';
import { motion, useReducedMotion } from 'motion/react';

const navigation = [
  { id: 'overview', label: '项目总览', icon: 'overview' },
  { id: 'inbox', label: '待我动作', icon: 'inbox' },
  { id: 'documents', label: '文档', icon: 'document' },
] as const;

export function AppShell({
  view,
  onNavigate,
  sourceLabel,
  projects,
  onOpen,
  actionCount,
  documentCount,
  toolbar,
  children,
}: {
  view: View;
  onNavigate: (view: View) => void;
  sourceLabel: string;
  projects: Project[];
  onOpen: OpenSource;
  actionCount: number;
  documentCount: number;
  toolbar: ReactNode;
  children: ReactNode;
}) {
  const reducedMotion = useReducedMotion();
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>
      <aside className="sidebar" aria-label="工作台">
        <div className="brand">Studiolo</div>
        <div className="workspace-source" title={sourceLabel}>
          <Icon name="folder" />
          <span>{sourceLabel}</span>
        </div>
        <nav aria-label="主导航">
          {navigation.map((item) => (
            <button
              key={item.id}
              className={`navigation-item ${view === item.id ? 'is-active' : ''}`}
              aria-current={view === item.id ? 'page' : undefined}
              aria-label={item.label}
              onClick={() => onNavigate(item.id)}
            >
              {view === item.id ? (
                <motion.span
                  className="navigation-highlight"
                  layoutId={reducedMotion ? undefined : 'workspace-navigation'}
                  transition={{ type: 'spring', stiffness: 420, damping: 42 }}
                  aria-hidden="true"
                />
              ) : null}
              <span className={`nav-symbol nav-symbol-${item.id}`}>
                <Icon name={item.icon} />
              </span>
              <span>{item.label}</span>
              {item.id !== 'overview' ? (
                <span className="nav-count" aria-hidden="true">
                  {item.id === 'inbox' ? actionCount : documentCount}
                </span>
              ) : null}
            </button>
          ))}
        </nav>
        {projects.length ? (
          <section className="sidebar-projects" aria-label="项目快捷入口">
            <h2>项目</h2>
            <ul>
              {projects.slice(0, 8).map((project) => (
                <li key={project.id}>
                  {project.source ? (
                    <button
                      onClick={() => onOpen(project.source!)}
                      aria-label={`${project.name}，打开项目来源`}
                    >
                      <Icon name="folder" />
                      <span>{project.name}</span>
                    </button>
                  ) : (
                    <span className="unfiled-shortcut">
                      <Icon name="folder" />
                      {project.name}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        <div className="sidebar-footer">
          <div>
            <Icon name="lock" />
            <span>只读副本</span>
          </div>
          <p>不修改源文件</p>
        </div>
      </aside>
      <div className="workspace">
        <header className="workspace-toolbar">
          <p className="workspace-breadcrumb">
            工作台 <span>/</span>{' '}
            <strong>
              {navigation.find((item) => item.id === view)?.label}
            </strong>
          </p>
          {toolbar}
        </header>
        <main id="main-content" className="main-content" tabIndex={-1}>
          {children}
        </main>
      </div>
      <footer className="page-footer">
        <Icon name="lock" />
        所有内容来自只读副本；不会修改笔记。
      </footer>
    </div>
  );
}
