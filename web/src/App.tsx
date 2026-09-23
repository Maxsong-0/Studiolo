import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { AppShell } from './components/AppShell';
import { DocumentDialog } from './components/DocumentDialog';
import { Filters } from './components/Filters';
import type { Project, SourceLocation, View } from './types';
import { useEditorNavigation } from './useEditorNavigation';
import { useViewLocation } from './useViewLocation';
import { useWorkbench } from './useWorkbench';
import { sortProjects } from './projectOrder';
import { QuickSwitcher } from './components/QuickSwitcher';
import { useQuickSwitcher } from './useQuickSwitcher';
import { CanonicalEditor } from './components/CanonicalEditor';
import { DraftShelf } from './components/DraftShelf';
import { useDraftCount } from './draftContext';
import { FileOperationsAccess } from './components/FileOperationsAccess';
import { CommandEntry } from './shell/CommandEntry';
import { StatusCapsule } from './shell/StatusCapsule';
import { useHealthStatus } from './shell/useHealthStatus';
import { CockpitView } from './views/CockpitView';
import { ProjectsView } from './views/ProjectsView';
import { DocumentsView } from './views/DocumentsView';
import { CollabView } from './views/CollabView';
import { PolicyView } from './views/PolicyView';

export function App({ account }: { account?: ReactNode }) {
  const { location, selectView, goTo } = useViewLocation();
  const view = location.view;
  const [query, setQuery] = useState('');
  const [lifecycle, setLifecycle] = useState('');
  const [source, setSource] = useState<SourceLocation | null>(null);
  const [draftsOpen, setDraftsOpen] = useState(false);
  const [statusOpen, setStatusOpen] = useState(false);
  const { count: draftCount } = useDraftCount();
  // 工作台先于本机健康发请求：认证名额只有两个（ADR-022），主内容排在前面。
  // 本机健康不依赖工作台快照：快照读不出来时，状态胶囊照样要能报出健康。
  const { data, loading, error, refresh, refreshLatest } = useWorkbench();
  const health = useHealthStatus();
  const editor = useEditorNavigation(refreshLatest);
  const editorPath = editor.current?.path ?? null;
  const setEditorPath = editor.open;
  const quick = useQuickSwitcher(
    Boolean(data) && !source && !editorPath && !draftsOpen && !loading,
  );
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
    if (loading) return;
    setSource(null);
    quick.close();
    refresh();
  }

  // 状态浮层的「刷新快照」：重读工作台快照，也重读本机健康；工作台先发。
  function refreshStatus() {
    if (loading) return;
    refreshSnapshot();
    health.reload();
  }

  function navigate(next: View) {
    selectView(next);
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }

  function enterProject(project: Project) {
    goTo({ view: 'projects', panel: 'operations', project: project.root });
    window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
  }

  const filters = (
    <Filters
      query={query}
      lifecycle={lifecycle}
      lifecycles={lifecycles}
      onQueryChange={setQuery}
      onLifecycleChange={setLifecycle}
      count={filtered.length}
    />
  );

  return (
    <AppShell
      view={view}
      onNavigate={navigate}
      actionCount={data ? data.inbox.length : null}
      documentCount={data?.counts.documents.value ?? null}
      status={
        <StatusCapsule
          open={statusOpen}
          onOpenChange={setStatusOpen}
          snapshot={
            data
              ? {
                  sourceLabel: data.source_label,
                  generation: data.generation,
                  freshness: data.freshness,
                }
              : null
          }
          refreshing={loading}
          failed={Boolean(error)}
          onVerify={refreshSnapshot}
          onRefresh={refreshStatus}
          health={health.status}
          diagnostics={diagnostics}
        />
      }
      command={<CommandEntry onOpen={quick.show} disabled={!data || loading} />}
      tools={
        <>
          {account}
          <FileOperationsAccess onChanged={refreshSnapshot} />
          {draftCount ? (
            <button
              className="outline-button draft-button"
              onClick={() => setDraftsOpen(true)}
            >
              内存草稿 {draftCount}
            </button>
          ) : null}
        </>
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
          正在读取工作台…
        </p>
      ) : null}
      {view === 'cockpit' && data ? (
        <CockpitView
          filters={filters}
          actions={actions}
          projects={filtered}
          diagnostics={diagnostics.length}
          onOpen={setSource}
          onEnterProject={enterProject}
          onViewProjects={() => navigate('projects')}
          onShowDiagnostics={() => setStatusOpen(true)}
        />
      ) : null}
      {view === 'projects' && data ? (
        <ProjectsView
          filters={filters}
          projects={filtered}
          allProjects={projects}
          generation={data.generation}
          refreshing={loading}
          onOpen={setSource}
          onRefresh={refreshSnapshot}
        />
      ) : null}
      {view === 'documents' && data ? (
        <DocumentsView
          projects={projects}
          generation={data.generation}
          onOpen={setSource}
          onRefresh={refreshSnapshot}
        />
      ) : null}
      {/* 协作与策略不依赖工作台快照：快照读不出来时也要能看黑板、发布或吊销。 */}
      {view === 'collab' ? <CollabView projects={projects} /> : null}
      {view === 'policy' ? <PolicyView /> : null}
      {source && data ? (
        <DocumentDialog
          key={`${source.path}:${data.generation}`}
          source={source}
          generation={data.generation}
          onClose={() => setSource(null)}
          onRefresh={refreshSnapshot}
          onEdit={(path) => {
            setSource(null);
            setEditorPath(path);
          }}
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
      {editorPath ? (
        <CanonicalEditor
          key={editor.current?.id}
          path={editorPath}
          expectedVault={editor.current?.vault}
          onRenamed={editor.onRenamed}
          onClose={() => {
            setEditorPath(null);
            refreshSnapshot();
          }}
        />
      ) : null}
      {draftsOpen ? (
        <DraftShelf
          onClose={() => setDraftsOpen(false)}
          onOpen={(path) => {
            setDraftsOpen(false);
            setEditorPath(path);
          }}
        />
      ) : null}
    </AppShell>
  );
}
