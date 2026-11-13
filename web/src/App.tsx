import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { AppShell } from './components/AppShell';
import { DocumentDialog } from './components/DocumentDialog';
import { Filters } from './components/Filters';
import type { SourceLocation, View } from './types';
import { useEditorNavigation } from './useEditorNavigation';
import { useViewLocation } from './useViewLocation';
import { useWorkbench } from './useWorkbench';
import { sortProjects } from './projectOrder';
import { CommandPalette } from './components/CommandPalette';
import type { CommandPaletteContext } from './components/CommandPalette';
import { focusCommandTarget } from './commands/focusTarget';
import { useCommandPalette } from './commands/useCommandPalette';
import { CanonicalEditor } from './components/CanonicalEditor';
import { DraftShelf } from './components/DraftShelf';
import { useDraftCount } from './draftContext';
import { FileOperationsAccess } from './components/FileOperationsAccess';
import type { FileOperationsEntry } from './components/FileOperationsAccess';
import { CommandEntry } from './shell/CommandEntry';
import { StatusCapsule } from './shell/StatusCapsule';
import { useHealthStatus } from './shell/useHealthStatus';
import { collabPanel } from './collab/panels';
import { KeyboardSheet } from './keys/KeyboardSheet';
import { KeyScopeContext, useGlobalKeys } from './keys/useGlobalKeys';
import {
  AutoRefreshContext,
  useAutoRefresh,
  useAutoRefreshReader,
} from './live/useAutoRefresh';
import { pendingCount } from './cockpit/pending';
import { useOverviewRead } from './cockpit/useOverviewRead';
import { usePolicyPendingRead } from './cockpit/usePolicyPendingRead';
import { CockpitView } from './views/CockpitView';
import { ProjectsView } from './views/ProjectsView';
import { DocumentsView } from './views/DocumentsView';
import { CollabView } from './views/CollabView';
import { RulesView } from './views/RulesView';
import { PolicyView } from './views/PolicyView';

export function App({ account }: { account?: ReactNode }) {
  const { location, selectView, goTo } = useViewLocation();
  const view = location.view;
  const [query, setQuery] = useState('');
  const [lifecycle, setLifecycle] = useState('');
  const [source, setSource] = useState<SourceLocation | null>(null);
  const [draftsOpen, setDraftsOpen] = useState(false);
  const [statusOpen, setStatusOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  // 顶栏「最近删除」入口：出现与否由文件操作接口决定，⌘K 面板据此收录「打开最近删除」。
  const [fileOperations, setFileOperations] =
    useState<FileOperationsEntry | null>(null);
  const { count: draftCount } = useDraftCount();
  // 工作台先于本机健康发请求：认证名额只有两个（ADR-022），主内容排在前面。
  // 本机健康不依赖工作台快照：快照读不出来时，状态胶囊照样要能报出健康。
  const { data, loading, error, refresh, refreshLatest, revalidate } =
    useWorkbench();
  const health = useHealthStatus();
  // 跨项目黑板汇总与策略草案列表全站各只读一份：快照到手时随同一次提交发出，每轮刷新结束重读（T-162、T-195）。
  // 顶栏「驾驶舱 N」角标与驾驶舱巨号都用 pendingCount 从这两份和同一份快照算，任何时刻都相等；
  // 驾驶舱经 props 拿同样两份，不再另读，离开驾驶舱时角标照样含黑板里等你的任务和策略里等你的提案。
  // 协作页的拓扑也用这一份汇总（T-196）：正在看拓扑时不等快照就读，快照读不出来也能看黑板；
  // 首次读取快照的那一轮不算「刷新」，免得拓扑先读到一次、快照到手时又被清掉重读。
  const onTopology =
    view === 'collab' && collabPanel(location.panel) === 'topology';
  const overview = useOverviewRead({
    ready: data !== null || onTopology,
    refreshing: loading && data !== null,
  });
  const policyPending = usePolicyPendingRead({
    ready: data !== null,
    refreshing: loading,
  });
  const editor = useEditorNavigation(refreshLatest);
  const editorPath = editor.current?.path ?? null;
  const setEditorPath = editor.open;
  const palette = useCommandPalette(
    Boolean(data) && !source && !editorPath && !draftsOpen && !loading,
  );
  // 自动刷新（T-196）：只在驾驶舱和协作拓扑上，而且没有开着会被打断的东西时才读。
  // 阅读器和 ⌘K 面板按快照代次取键，底下换了快照会把它们整个重挂；编辑器和草稿架开着时人在干别的。
  // 三路按主次登记：快照、黑板汇总、策略草案；驾驶舱自己的健康读取排在它们后面（CockpitView 登记）。
  const auto = useAutoRefresh({
    active:
      (view === 'cockpit' || onTopology) &&
      !source &&
      !editorPath &&
      !draftsOpen &&
      !palette.open,
    reading: loading,
    failed: Boolean(error),
  });
  useAutoRefreshReader(auto, revalidate, 0);
  useAutoRefreshReader(auto, overview.revalidate, 1);
  useAutoRefreshReader(auto, policyPending.revalidate, 2);
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
    palette.close();
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

  // 单键快捷键（T-197）：G 再按字母和点顶栏导航一样；「?」打开一览。各视图自己的键位经它登记（驾驶舱队列的 J／K／Enter／H）。
  const keys = useGlobalKeys({
    onGo: navigate,
    onShowSheet: () => setShortcutsOpen(true),
  });

  // ⌘K 的「查看全部项目」：先清掉项目视图上次留下的筛选，打开后看到的才是全部（T-117）。
  function viewAllProjects() {
    setQuery('');
    setLifecycle('');
    navigate('projects');
  }

  // ⌘K 面板执行命令用的动作：跳转与顶栏导航一样回到页首；其余沿用本页已有的入口。
  const paletteContext: CommandPaletteContext = {
    location,
    projects,
    draftCount,
    fileOperations,
    goTo: (target) => {
      goTo(target);
      window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
    },
    viewAllProjects: () => {
      viewAllProjects();
      window.scrollTo({ top: 0, left: 0, behavior: 'instant' });
    },
    refreshSnapshot,
    openSource: setSource,
    openDrafts: () => setDraftsOpen(true),
    openRecentlyDeleted: () => fileOperations?.open(),
    openStatus: () => setStatusOpen(true),
    openShortcuts: () => setShortcutsOpen(true),
    focusTarget: (target) => {
      focusCommandTarget(target);
    },
  };

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

  const shell = (
    <AppShell
      view={view}
      onNavigate={navigate}
      sourceLabel={data?.source_label ?? null}
      actionCount={pendingCount(data, overview.state, policyPending.state)}
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
      command={
        <CommandEntry onOpen={palette.show} disabled={!data || loading} />
      }
      tools={
        <>
          {account}
          <FileOperationsAccess
            onChanged={refreshSnapshot}
            onEntryChange={setFileOperations}
          />
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
          workbench={data}
          refreshing={loading}
          overview={overview}
          policy={policyPending}
          diagnostics={diagnostics.length}
          onOpen={setSource}
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
      {/* 协作、规则与策略不依赖工作台快照：快照读不出来时也要能看黑板、改规则、发布或吊销。 */}
      {view === 'collab' ? (
        <CollabView projects={projects} overview={overview} />
      ) : null}
      {view === 'rules' ? <RulesView /> : null}
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
      {palette.open && data ? (
        <CommandPalette
          key={data.generation}
          context={paletteContext}
          generation={data.generation}
          onClose={palette.close}
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
      {shortcutsOpen ? (
        <KeyboardSheet
          preference={keys.preference}
          onClose={() => setShortcutsOpen(false)}
        />
      ) : null}
    </AppShell>
  );
  // 调度器交给下面：驾驶舱登记自己的健康读取，「读于 N 秒前」订它的状态（T-196）。
  // 外壳先放进变量再包一层，免得整段 JSX 为了多一层标签全部重排缩进。
  return (
    <AutoRefreshContext.Provider value={auto}>
      <KeyScopeContext.Provider value={keys}>{shell}</KeyScopeContext.Provider>
    </AutoRefreshContext.Provider>
  );
}
