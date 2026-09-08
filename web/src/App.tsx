import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { AppShell } from './components/AppShell';
import { DocumentDialog } from './components/DocumentDialog';
import { Documents } from './components/Documents';
import { Filters } from './components/Filters';
import { Icon } from './components/Icon';
import { Inbox } from './components/Inbox';
import { ProjectTable } from './components/ProjectTable';
import type { SourceLocation, View } from './types';
import { useWorkbench } from './useWorkbench';
import { sortProjects } from './projectOrder';
import { QuickSwitcher } from './components/QuickSwitcher';
import { useQuickSwitcher } from './useQuickSwitcher';
import { Diagnostics } from './components/Diagnostics';

const headings = {
  overview: ['项目总览', '项目状态与待办，回到原文核对。'],
  inbox: ['待我动作', '待你验收、拍板与重启的事项，回到原文核对。'],
  documents: ['文档', '搜索笔记或浏览目录与附件，所有内容只读。'],
} satisfies Record<View, [string, string]>;

export function App({ account }: { account?: ReactNode }) {
  const [view, setView] = useState<View>('overview');
  const [query, setQuery] = useState('');
  const [lifecycle, setLifecycle] = useState('');
  const [source, setSource] = useState<SourceLocation | null>(null);
  const { data, loading, error, refresh } = useWorkbench();
  const quick = useQuickSwitcher(Boolean(data) && !source && !loading);
  const projects = useMemo(() => sortProjects(data?.projects ?? []), [data]);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filtered = projects.filter(
    (project) =>
      `${project.name} ${project.root}`
        .toLocaleLowerCase()
        .includes(normalizedQuery) &&
      (!lifecycle ||
        (lifecycle === '__unknown'
          ? !project.lifecycle.value
          : project.lifecycle.value === lifecycle)),
  );
  const visibleProjectIds = new Set(filtered.map((project) => project.id));
  const actions = (data?.inbox ?? []).filter((action) =>
    visibleProjectIds.has(action.project_id),
  );
  const lifecycles = [
    ...new Set(
      projects.flatMap((project) =>
        project.lifecycle.value ? [project.lifecycle.value] : [],
      ),
    ),
  ];
  const diagnostics = [
    ...new Set([
      ...(data?.diagnostics ?? []),
      ...projects.flatMap((project) =>
        project.diagnostics.map((message) => `${project.name}：${message}`),
      ),
    ]),
  ];

  function refreshSnapshot() {
    setSource(null);
    quick.close();
    refresh();
  }

  function navigate(next: View) {
    setView(next);
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }

  return (
    <AppShell
      view={view}
      onNavigate={navigate}
      sourceLabel={data?.source_label ?? (loading ? '读取中' : '未连接')}
      projects={projects}
      onOpen={setSource}
      actionCount={data?.inbox.length ?? 0}
      documentCount={data?.documents.length ?? 0}
      toolbar={
        <div className="page-actions">
          <button
            className="outline-button quick-button"
            onClick={quick.show}
            disabled={!data || loading}
            aria-keyshortcuts="Meta+K Control+K"
          >
            <Icon name="search" />
            <span>快速切换</span>
            <kbd>⌘K</kbd>
          </button>
          <button
            className="outline-button refresh-button"
            onClick={refreshSnapshot}
            disabled={loading}
          >
            <Icon name="refresh" />
            {loading && data ? '刷新中' : '刷新'}
          </button>
          {account}
        </div>
      }
    >
      {error ? (
        <div className="error-feedback" role="alert">
          <p>
            {data ? '刷新失败，仍显示上次快照。' : '无法读取工作台。'}
            {error}
          </p>
          <button className="text-button" onClick={refreshSnapshot}>
            重试
          </button>
        </div>
      ) : null}
      {!data && loading ? (
        <p className="empty-state" role="status">
          正在读取只读副本…
        </p>
      ) : null}
      <div className={`view-layout view-${view}`}>
        <section className="view-primary" aria-label={headings[view][0]}>
          <header className="page-header">
            <h1>{headings[view][0]}</h1>
            <p>{headings[view][1]}</p>
          </header>
          <div className="view-content" key={view}>
            {data ? (
              <>
                {view !== 'documents' ? (
                  <Filters
                    query={query}
                    lifecycle={lifecycle}
                    lifecycles={lifecycles}
                    onQueryChange={setQuery}
                    onLifecycleChange={setLifecycle}
                    count={filtered.length}
                  />
                ) : null}
                {view === 'overview' ? (
                  <ProjectTable projects={filtered} onOpen={setSource} />
                ) : null}
                {view === 'inbox' ? (
                  <Inbox actions={actions} onOpen={setSource} heading={false} />
                ) : null}
                {view === 'documents' ? (
                  <Documents
                    key={data.generation}
                    projects={projects}
                    generation={data.generation}
                    onOpen={setSource}
                    onRefresh={refreshSnapshot}
                  />
                ) : null}
              </>
            ) : null}
            {view !== 'overview' ? (
              <Diagnostics messages={diagnostics} />
            ) : null}
          </div>
        </section>
        {view === 'overview' && data ? (
          <aside className="overview-context" aria-label="项目上下文">
            <Inbox
              actions={actions.slice(0, 3)}
              onOpen={setSource}
              onViewAll={() => navigate('inbox')}
            />
            <Diagnostics messages={diagnostics} />
          </aside>
        ) : null}
      </div>
      {source && data ? (
        <DocumentDialog
          key={`${source.path}:${data.generation}`}
          source={source}
          generation={data.generation}
          onClose={() => setSource(null)}
          onRefresh={refreshSnapshot}
        />
      ) : null}
      {quick.open && data ? (
        <QuickSwitcher
          key={data.generation}
          projects={projects}
          generation={data.generation}
          onOpen={(location) => {
            quick.close();
            setSource(location);
          }}
          onClose={quick.close}
          onRefresh={refreshSnapshot}
        />
      ) : null}
    </AppShell>
  );
}
