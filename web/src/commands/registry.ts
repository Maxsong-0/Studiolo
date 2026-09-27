/**
 * ⌘K 动作面板的命令清单（设计规范第 8 节）。只收录接口真实支持的动作：
 * 切换视图与协作子标签、打开项目与项目来源、刷新快照、内存草稿、最近删除、状态详情、
 * 跳到黑板任务、执行候选、策略待处理队列、新建规则；文档组来自全文搜索。
 * 不可逆或需二次认证的动作（发布/回滚策略、删除文件、落地候选产物）只做「执行预览」：
 * 回车只把人带到拥有该动作的界面，面板自己绝不调接口，确认仍在原界面完成。
 */
import type { BoardOverview, BoardOverviewStatus } from '../boardOverviewModel';
import type { Project, SearchHit, SourceLocation, View } from '../types';
import type { ViewLocation, ViewSlot } from '../viewLocation';
import { COMMAND_TARGETS } from './focusTarget';
import type { CommandTarget } from './focusTarget';

/** 三组，顺序固定：动作 · 跳转 · 文档。 */
export type CommandGroup = 'action' | 'navigate' | 'document';

export const COMMAND_GROUPS: readonly { id: CommandGroup; title: string }[] = [
  { id: 'action', title: '动作' },
  { id: 'navigate', title: '跳转' },
  { id: 'document', title: '文档' },
];

/** 跳转目标，与 useViewLocation().goTo 同形：视图加可选的项目／面板／条目格。 */
export type CommandLocation = { view: View } & Partial<
  Record<ViewSlot, string | null>
>;

/** 命令落到的界面；`focus` 是那个界面上要聚焦的控件（见 focusTarget.ts 的约定）。 */
export type CommandSurface = CommandLocation & { focus?: CommandTarget };

export interface CommandContext {
  /** 当前位置：在协作页里切子标签时沿用已选的项目，与页内子标签的行为一致。 */
  location: ViewLocation;
  /** 工作台快照里的全部项目（已排序）。 */
  projects: readonly Project[];
  /** 黑板汇总；读取中、未启用或读取失败时为 null，此时不列黑板任务。 */
  overview: BoardOverview | null;
  /** 本页内存草稿篇数；为 0 时不列「打开内存草稿」，与顶栏入口的出现条件相同。 */
  draftCount: number;
  /** 文件操作入口：顶栏「最近删除」在不在，以及文件操作是否可用（可用才能删文件）。 */
  fileOperations: { deletable: boolean } | null;
  goTo(target: CommandLocation): void;
  /** 打开项目视图并先清掉上次留下的筛选：「全部 N 个项目」写的总数与打开后看到的行数才对得上（T-117）。 */
  viewAllProjects(): void;
  refreshSnapshot(): void;
  openSource(source: SourceLocation): void;
  openDrafts(): void;
  openRecentlyDeleted(): void;
  openStatus(): void;
  /** 跳转之后尝试把焦点放到目标控件上；找不到就不动。 */
  focusTarget(target: CommandTarget): void;
}

/** 同类实体（项目、项目来源、黑板任务）：面板里每类只列前几条，超出给「全部 N …」入口（见 arrange.ts）。 */
export type CommandFamily = 'project' | 'source' | 'task';

/** 不可逆动作的预览：说明这个动作做什么、要经过哪些确认，以及回车会去哪里。 */
export interface IrreversiblePreview {
  preview: string;
  surface: CommandSurface;
  /** 拥有这个动作的界面，写进「按 ↵ 前往…」。 */
  surfaceLabel: string;
}

interface CommandBase {
  /** 面板内唯一。 */
  id: string;
  group: CommandGroup;
  label: string;
  /** 搜索用的别名：同义词、英文名、路径、编号。 */
  keywords: readonly string[];
  /** 名字后面的灰色说明，也参与匹配。 */
  hint?: string;
  family?: CommandFamily;
}

/** 三选一：`run` 直接做（只读或本页状态），`target` 跳到某处，`irreversible` 只预览并带路。 */
export type Command = CommandBase &
  (
    | {
        run: (ctx: CommandContext) => void;
        target?: undefined;
        irreversible?: undefined;
      }
    | {
        target: (ctx: CommandContext) => CommandSurface;
        run?: undefined;
        irreversible?: undefined;
      }
    | {
        irreversible: IrreversiblePreview;
        run?: undefined;
        target?: undefined;
      }
  );

/** 已结束的四个任务状态（与 boardOverviewModel 的 terminal 表一致）；面板只列未结束的任务。 */
const FINISHED: ReadonlySet<BoardOverviewStatus> = new Set([
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
]);

/** 任务状态的人话，与协作页「任务与回执」的写法一致。 */
const TASK_STATUS: Record<BoardOverviewStatus, string> = {
  draft: '草稿，尚未排队',
  queued: '待认领',
  claimed: '已认领',
  running: '进行中',
  committing: '写回中',
  cancel_requested: '等待停止回执',
  needs_review: '待审阅',
  needs_revision: '等待修订',
  needs_reconfirm: '等待重新确认',
  approved: '已审阅通过',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已停止',
  abandoned: '已放弃',
};

/** 在协作页里切子标签沿用所选项目；从别的视图过来不带项目（各子标签默认取第一个项目）。 */
function collabProject(ctx: CommandContext): string | null {
  return ctx.location.view === 'collab' ? ctx.location.project : null;
}

const VIEW_HINT = '切换视图';
const VIEW_WORDS = ['切换', '跳转', '视图', 'view'];

function viewCommands(): Command[] {
  const views: {
    id: string;
    label: string;
    keywords: string[];
    target: (ctx: CommandContext) => CommandSurface;
  }[] = [
    {
      id: 'view:cockpit',
      label: '驾驶舱',
      keywords: ['cockpit', 'home', '首页', '总览', '待我动作'],
      target: () => ({ view: 'cockpit' }),
    },
    {
      id: 'view:projects',
      label: '项目',
      keywords: ['projects', '项目明细'],
      target: () => ({ view: 'projects' }),
    },
    {
      id: 'view:documents',
      label: '文档',
      keywords: ['documents', 'docs', '目录树', '阅读器'],
      target: () => ({ view: 'documents' }),
    },
    {
      id: 'view:collab:topology',
      label: '协作 › 拓扑',
      keywords: ['collab', 'topology', '写域', '占用'],
      target: (ctx) => ({ view: 'collab', project: collabProject(ctx) }),
    },
    {
      id: 'view:collab:board',
      label: '协作 › 任务与回执',
      keywords: ['collab', 'board', '黑板', '派发', 'receipt'],
      target: (ctx) => ({
        view: 'collab',
        panel: 'board',
        project: collabProject(ctx),
      }),
    },
    {
      id: 'view:collab:timeline',
      label: '协作 › 时间线',
      keywords: ['collab', 'timeline', 'replay', '回放'],
      target: (ctx) => ({
        view: 'collab',
        panel: 'timeline',
        project: collabProject(ctx),
      }),
    },
    {
      id: 'view:collab:execution',
      label: '协作 › 执行',
      keywords: ['collab', 'execution', '冻结', '候选'],
      target: (ctx) => ({
        view: 'collab',
        panel: 'execution',
        project: collabProject(ctx),
      }),
    },
    {
      id: 'view:rules',
      label: '规则',
      keywords: ['rules', '规则母本', 'CLAUDE.md', '读权限', '谁能读'],
      target: () => ({ view: 'rules' }),
    },
    {
      id: 'view:policy',
      label: '策略',
      keywords: ['policy', '规则', '能力集', 'skills'],
      target: () => ({ view: 'policy' }),
    },
  ];
  return views.map((view) => ({
    ...view,
    group: 'navigate',
    hint: VIEW_HINT,
    keywords: [...view.keywords, ...VIEW_WORDS],
  }));
}

function actionCommands(ctx: CommandContext): Command[] {
  const commands: Command[] = [
    {
      id: 'action:status',
      group: 'action',
      label: '打开状态详情',
      hint: '新鲜度、本机健康与数据提示',
      keywords: ['状态', '健康', '新鲜度', '数据提示', 'status', 'health'],
      run: (context) => context.openStatus(),
    },
    {
      id: 'action:refresh',
      group: 'action',
      label: '刷新快照',
      hint: '重新读取工作台',
      keywords: ['刷新', '重新读取', '工作台', 'refresh', 'reload'],
      run: (context) => context.refreshSnapshot(),
    },
  ];
  if (ctx.draftCount > 0)
    commands.push({
      id: 'action:drafts',
      group: 'action',
      label: '打开内存草稿',
      hint: `本页 ${ctx.draftCount} 篇`,
      keywords: ['草稿', '未保存', 'draft'],
      run: (context) => context.openDrafts(),
    });
  if (ctx.fileOperations)
    commands.push({
      id: 'action:recently-deleted',
      group: 'action',
      label: '打开最近删除',
      hint: '恢复删除的文件，核对进行中的文件操作',
      keywords: ['最近删除', '恢复', '回收站', 'deleted', 'trash', 'restore'],
      run: (context) => context.openRecentlyDeleted(),
    });
  commands.push(
    {
      id: 'action:execution-candidates',
      group: 'action',
      label: '打开执行候选',
      hint: '协作 › 执行：逐项审阅候选产物',
      keywords: ['执行', '候选', '候选产物', '审阅', 'execution', 'candidate'],
      // 与「协作 › 执行」同一个标签页，另把焦点带到候选审阅区（界面配合时生效）。
      target: (context) => ({
        view: 'collab',
        panel: 'execution',
        project: collabProject(context),
        focus: COMMAND_TARGETS.executionReview,
      }),
    },
    {
      id: 'action:policy-queue',
      group: 'action',
      label: '打开策略待处理队列',
      hint: '策略 › 待处理草案',
      keywords: ['策略', '待处理', '规则草案', '队列', 'desired', 'policy'],
      target: () => ({
        view: 'policy',
        item: 'desired',
        focus: COMMAND_TARGETS.policyQueue,
      }),
    },
    {
      id: 'action:rule-new',
      group: 'action',
      label: '新建规则',
      hint: '规则 › 编辑：写好正文后保存为规则草案',
      keywords: ['新建', '规则', '规则母本', 'new rule', 'rules'],
      // 只打开空白的编辑面板（item 为空即新建）；保存只形成草案，发布仍在那里经二次认证完成。
      target: () => ({ view: 'rules', panel: 'edit' }),
    },
    preview({
      id: 'action:policy-publish',
      label: '发布策略',
      keywords: ['发布', '上线', '生效', 'publish', 'activate', 'desired'],
      preview:
        '发布会把一份已校验通过的规则草案（desired）变成新的生效版本（effective）。先在待处理队列里选定草案、逐条确认扩大项，再经一次性验证码（One-time Code）二次认证。',
      surface: {
        view: 'policy',
        item: 'desired',
        focus: COMMAND_TARGETS.policyQueue,
      },
      surfaceLabel: '策略 › 待处理队列',
    }),
    preview({
      id: 'action:policy-rollback',
      label: '回滚策略',
      keywords: ['回滚', '撤回版本', 'rollback', 'version'],
      preview:
        '回滚会把选中的历史版本重新发布为新的生效版本，同样要经一次性验证码（One-time Code）二次认证。先在「版本与回滚」里核对那一版的正文，再逐版确认。',
      surface: { view: 'policy', focus: COMMAND_TARGETS.policyVersions },
      surfaceLabel: '策略 › 版本与回滚',
    }),
  );
  if (ctx.fileOperations?.deletable)
    commands.push(
      preview({
        id: 'action:file-delete',
        label: '删除文件',
        keywords: ['删除', '移除', 'delete', 'remove'],
        preview:
          '删除只针对某一个已保存的文件：先在文档里找到并打开它、进入编辑，处理完草稿后在「删除文件」处勾选确认。删除后可在「最近删除」恢复原路径。',
        surface: {
          view: 'documents',
          focus: COMMAND_TARGETS.documentsSearch,
        },
        surfaceLabel: '文档',
      }),
    );
  commands.push(
    preview({
      id: 'action:execution-land',
      label: '落地候选产物',
      keywords: ['落地', '应用补丁', '候选', 'land', 'patch', 'candidate'],
      preview:
        '落地会把候选补丁应用到目标仓库：先在协作 › 执行里选定任务，核对目标仓库完整 HEAD 与 candidate.patch 的 SHA-256，再经二次认证落地。',
      surface: {
        view: 'collab',
        panel: 'execution',
        project: collabProject(ctx),
        focus: COMMAND_TARGETS.executionLanding,
      },
      surfaceLabel: '协作 › 执行',
    }),
  );
  return commands;
}

function preview({
  id,
  label,
  keywords,
  ...irreversible
}: {
  id: string;
  label: string;
  keywords: string[];
} & IrreversiblePreview): Command {
  // 说明里不写「执行预览」四个字：说明参与匹配，写了会让输入「执行」时四条预览全冒出来。
  // 预览身份由行尾的红字「预览」与选中后的红色虚线框表达。
  return {
    id,
    group: 'action',
    label,
    hint: `确认在${irreversible.surfaceLabel}完成`,
    keywords: [...keywords, '预览', '不可逆', '二次认证'],
    irreversible,
  };
}

function projectCommands(ctx: CommandContext): Command[] {
  const open: Command[] = ctx.projects.map((project) => ({
    id: `project:${project.id}`,
    group: 'navigate',
    family: 'project',
    label: `项目 › ${project.name}`,
    hint: '进入运作',
    keywords: ['打开项目', '运作', 'project', project.root, project.id],
    target: () => ({
      view: 'projects',
      panel: 'operations',
      project: project.root,
    }),
  }));
  const sources = ctx.projects.flatMap((project): Command[] => {
    const source = project.source;
    if (!source) return [];
    return [
      {
        id: `source:${project.id}`,
        group: 'navigate',
        family: 'source',
        label: `项目来源 › ${project.name}`,
        hint: `${source.path} · 第 ${source.line} 行`,
        keywords: ['打开项目来源', '来源', 'source', project.root, project.id],
        run: (context) => context.openSource(source),
      },
    ];
  });
  return [...open, ...sources];
}

function taskCommands(ctx: CommandContext): Command[] {
  if (!ctx.overview) return [];
  const names = new Map(
    ctx.projects.map((project) => [project.root, project.name]),
  );
  return ctx.overview.items.flatMap((item): Command[] =>
    FINISHED.has(item.status)
      ? []
      : [
          {
            id: `task:${item.project}:${item.id}`,
            group: 'navigate',
            family: 'task',
            label: `协作 › ${item.title}`,
            hint: `${names.get(item.project) ?? item.project} · ${TASK_STATUS[item.status]}`,
            keywords: [
              '黑板任务',
              '任务',
              'task',
              item.id,
              item.project,
              item.status,
              item.adapter,
            ],
            // 选中该任务并进拓扑：拓扑默认看全部项目，条目格记任务 id。
            target: () => ({ view: 'collab', item: item.id }),
          },
        ],
  );
}

/** 按上下文生成完整清单（未过滤、未截断），顺序即同分时的排列顺序。 */
export function buildCommands(ctx: CommandContext): Command[] {
  return [
    ...actionCommands(ctx),
    ...viewCommands(),
    ...projectCommands(ctx),
    ...taskCommands(ctx),
  ];
}

/** 一条全文搜索命中 → 文档组的一行，回车打开原文并定位到命中行。 */
export function documentCommand(hit: SearchHit, index: number): Command {
  const snippet =
    hit.kind === 'content' ? hit.snippet.replace(/\s+/gu, ' ').trim() : '';
  return {
    id: `document:${index}:${hit.path}:${hit.line}`,
    group: 'document',
    label: hit.title,
    hint: [snippet, `${hit.path} · 第 ${hit.line} 行`]
      .filter(Boolean)
      .join('　'),
    keywords: [],
    run: (ctx) => ctx.openSource({ path: hit.path, line: hit.line }),
  };
}

/** 执行一条命令。不可逆动作只跳到拥有它的界面并尝试聚焦那个控件，不调任何接口。 */
export function executeCommand(command: Command, ctx: CommandContext): void {
  if (command.run) {
    command.run(ctx);
    return;
  }
  const surface = command.irreversible
    ? command.irreversible.surface
    : command.target(ctx);
  const { focus, ...location } = surface;
  ctx.goTo(location);
  if (focus) ctx.focusTarget(focus);
}
