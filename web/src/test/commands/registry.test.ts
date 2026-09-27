import { beforeEach, describe, expect, it, vi } from 'vitest';
import { parseBoardOverview } from '../../boardOverviewModel';
import { COMMAND_TARGETS } from '../../commands/focusTarget';
import { arrangeCommands, FAMILY_LIMIT } from '../../commands/arrange';
import {
  buildCommands,
  documentCommand,
  executeCommand,
} from '../../commands/registry';
import type { Command, CommandContext } from '../../commands/registry';
import { project } from '../fixtures';
import {
  manyProjects,
  overviewResponse,
  paletteContext,
  taskId,
} from './paletteFixtures';

const fetchMock = vi.fn<typeof fetch>();
beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});

function byId(commands: readonly Command[], id: string) {
  const command = commands.find((entry) => entry.id === id);
  if (!command) throw new Error(`缺少命令 ${id}`);
  return command;
}

/** 执行一条命令，返回它对上下文做了什么（只记被调用的动作）。 */
function effects(command: Command, context: CommandContext) {
  executeCommand(command, context);
  const calls: Record<string, unknown[][]> = {};
  for (const name of [
    'goTo',
    'viewAllProjects',
    'refreshSnapshot',
    'openSource',
    'openDrafts',
    'openRecentlyDeleted',
    'openStatus',
    'focusTarget',
  ] as const) {
    const spy = vi.mocked(context[name]);
    if (spy.mock.calls.length) calls[name] = spy.mock.calls;
  }
  return calls;
}

describe('命令清单：每条命令的目标或预览', () => {
  it('动作组：直接做的只调本页已有入口，跳转的只写位置', () => {
    const commands = buildCommands(
      paletteContext({
        draftCount: 2,
        fileOperations: { deletable: true },
      }).context,
    );
    const run = (id: string) => {
      const { context } = paletteContext();
      return effects(byId(commands, id), context);
    };
    expect(run('action:status')).toEqual({ openStatus: [[]] });
    expect(run('action:refresh')).toEqual({ refreshSnapshot: [[]] });
    expect(run('action:drafts')).toEqual({ openDrafts: [[]] });
    expect(run('action:recently-deleted')).toEqual({
      openRecentlyDeleted: [[]],
    });
    expect(run('action:execution-candidates')).toEqual({
      goTo: [[{ view: 'collab', panel: 'execution', project: null }]],
      focusTarget: [[COMMAND_TARGETS.executionReview]],
    });
    expect(run('action:policy-queue')).toEqual({
      goTo: [[{ view: 'policy', item: 'desired' }]],
      focusTarget: [[COMMAND_TARGETS.policyQueue]],
    });
    expect(byId(commands, 'action:drafts').hint).toBe('本页 2 篇');
  });

  it('跳转组：六个视图与协作四个子标签，协作页内切换沿用所选项目', () => {
    const commands = buildCommands(paletteContext().context);
    const targets = (location: CommandContext['location']) =>
      Object.fromEntries(
        commands
          .filter((command) => command.id.startsWith('view:'))
          .map((command) => {
            const { context } = paletteContext({ location });
            executeCommand(command, context);
            return [
              `${command.label}`,
              vi.mocked(context.goTo).mock.calls[0][0],
            ];
          }),
      );
    expect(
      targets({ view: 'cockpit', project: null, panel: null, item: null }),
    ).toEqual({
      驾驶舱: { view: 'cockpit' },
      项目: { view: 'projects' },
      文档: { view: 'documents' },
      '协作 › 拓扑': { view: 'collab', project: null },
      '协作 › 任务与回执': { view: 'collab', panel: 'board', project: null },
      '协作 › 时间线': { view: 'collab', panel: 'timeline', project: null },
      '协作 › 执行': { view: 'collab', panel: 'execution', project: null },
      规则: { view: 'rules' },
      策略: { view: 'policy' },
    });
    // 已在协作页看某个项目：换子标签不丢项目，条目格按新子标签重新解释所以清掉。
    expect(
      targets({
        view: 'collab',
        project: '项目/星舟',
        panel: 'board',
        item: 'x',
      })['协作 › 时间线'],
    ).toEqual({ view: 'collab', panel: 'timeline', project: '项目/星舟' });
    // 从别的视图带着项目格过来也不串：项目格按视图解释。
    expect(
      targets({
        view: 'projects',
        project: '项目/星舟',
        panel: 'operations',
        item: null,
      })['协作 › 执行'],
    ).toEqual({ view: 'collab', panel: 'execution', project: null });
  });

  it('项目与项目来源：打开项目进运作，来源打开原文；没有来源的项目不列来源', () => {
    const projects = manyProjects(2, [1]);
    const commands = buildCommands(paletteContext({ projects }).context);
    const open = byId(commands, 'project:p00');
    expect(open.label).toBe('项目 › 项目00');
    const { context } = paletteContext();
    expect(effects(open, context)).toEqual({
      goTo: [
        [{ view: 'projects', panel: 'operations', project: '项目/项目00' }],
      ],
    });
    const source = byId(commands, 'source:p00');
    expect(source.label).toBe('项目来源 › 项目00');
    expect(effects(source, paletteContext().context)).toEqual({
      openSource: [[{ path: '项目/项目00/00-总览/INDEX.md', line: 1 }]],
    });
    expect(commands.some((command) => command.id === 'source:p01')).toBe(false);
    expect(commands.some((command) => command.id === 'project:p01')).toBe(true);
  });

  it('黑板任务来自汇总：只列未结束的，选中并进拓扑', () => {
    const overview = parseBoardOverview(
      overviewResponse([
        {
          id: taskId('A'),
          title: '前端全面重设计',
          project: '项目/星舟',
          status: 'queued',
        },
        {
          id: taskId('B'),
          title: '已经收尾的盘点',
          project: '项目/星舟',
          status: 'succeeded',
        },
        {
          id: taskId('C'),
          title: '别处的任务',
          project: '项目/未知库',
          status: 'needs_review',
        },
      ]),
      24,
    );
    const commands = buildCommands(paletteContext({ overview }).context);
    const tasks = commands.filter((command) => command.family === 'task');
    expect(tasks.map((command) => [command.label, command.hint])).toEqual([
      ['协作 › 前端全面重设计', '星舟 · 待认领'],
      // 项目不在工作台快照里就显示原路径，不猜名字。
      ['协作 › 别处的任务', '项目/未知库 · 待审阅'],
    ]);
    expect(effects(tasks[0], paletteContext().context)).toEqual({
      goTo: [[{ view: 'collab', item: taskId('A') }]],
    });
    // 汇总没取到（读取中、未启用、失败）就不列任务。
    expect(
      buildCommands(paletteContext().context).some(
        (command) => command.family === 'task',
      ),
    ).toBe(false);
  });

  it('文档命中：打开原文并定位到命中行', () => {
    const command = documentCommand(
      {
        path: '项目/星舟/00-总览/INDEX.md',
        title: '星舟总览',
        line: 12,
        snippet: '  这一行\n写着 关键词 ',
        kind: 'content',
      },
      0,
    );
    expect(command.group).toBe('document');
    expect(command.hint).toBe(
      '这一行 写着 关键词　项目/星舟/00-总览/INDEX.md · 第 12 行',
    );
    expect(effects(command, paletteContext().context)).toEqual({
      openSource: [[{ path: '项目/星舟/00-总览/INDEX.md', line: 12 }]],
    });
  });
});

describe('不可逆动作只预览并带路', () => {
  const surfaces = {
    'action:policy-publish': {
      goTo: [[{ view: 'policy', item: 'desired' }]],
      focusTarget: [[COMMAND_TARGETS.policyQueue]],
    },
    'action:policy-rollback': {
      goTo: [[{ view: 'policy' }]],
      focusTarget: [[COMMAND_TARGETS.policyVersions]],
    },
    'action:file-delete': {
      goTo: [[{ view: 'documents' }]],
      focusTarget: [[COMMAND_TARGETS.documentsSearch]],
    },
    'action:execution-land': {
      goTo: [[{ view: 'collab', panel: 'execution', project: null }]],
      focusTarget: [[COMMAND_TARGETS.executionLanding]],
    },
  };

  it('发布、回滚、删文件、落地四条只带人到拥有它的界面，零请求、不碰其它动作', () => {
    const { context } = paletteContext({ fileOperations: { deletable: true } });
    const commands = buildCommands(context);
    const irreversible = commands.filter((command) => command.irreversible);
    expect(irreversible.map((command) => command.id)).toEqual(
      Object.keys(surfaces),
    );
    for (const command of irreversible) {
      const fresh = paletteContext().context;
      expect(effects(command, fresh)).toEqual(
        surfaces[command.id as keyof typeof surfaces],
      );
      expect(command.irreversible?.preview).toBeTruthy();
      expect(command.hint).toBe(
        `确认在${command.irreversible?.surfaceLabel}完成`,
      );
    }
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('在协作页里预览落地，沿用所选项目', () => {
    const location = {
      view: 'collab' as const,
      project: '项目/星舟',
      panel: 'execution',
      item: 'T-1',
    };
    const commands = buildCommands(paletteContext({ location }).context);
    const { context } = paletteContext({ location });
    expect(effects(byId(commands, 'action:execution-land'), context)).toEqual({
      goTo: [[{ view: 'collab', panel: 'execution', project: '项目/星舟' }]],
      focusTarget: [[COMMAND_TARGETS.executionLanding]],
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe('只收录真实可用的动作', () => {
  it('没有草稿不列草稿；文件操作入口不在不列最近删除与删文件；关闭时只能看最近删除', () => {
    const ids = (overrides: Partial<CommandContext>) =>
      buildCommands(paletteContext(overrides).context).map(
        (command) => command.id,
      );
    const bare = ids({});
    expect(bare).not.toContain('action:drafts');
    expect(bare).not.toContain('action:recently-deleted');
    expect(bare).not.toContain('action:file-delete');
    const pending = ids({ fileOperations: { deletable: false } });
    expect(pending).toContain('action:recently-deleted');
    expect(pending).not.toContain('action:file-delete');
    const ready = ids({ draftCount: 1, fileOperations: { deletable: true } });
    expect(ready).toContain('action:drafts');
    expect(ready).toContain('action:recently-deleted');
    expect(ready).toContain('action:file-delete');
  });

  it('完整清单只有这些种类：没有合并 PR 之类 Studiolo 没有接口的动作', () => {
    const commands = buildCommands(
      paletteContext({
        draftCount: 1,
        fileOperations: { deletable: true },
      }).context,
    );
    expect(
      commands
        .filter((command) => !command.family)
        .map((command) => command.label),
    ).toEqual([
      '打开状态详情',
      '刷新快照',
      '打开内存草稿',
      '打开最近删除',
      '打开执行候选',
      '打开策略待处理队列',
      '新建规则',
      '发布策略',
      '回滚策略',
      '删除文件',
      '落地候选产物',
      '驾驶舱',
      '项目',
      '文档',
      '协作 › 拓扑',
      '协作 › 任务与回执',
      '协作 › 时间线',
      '协作 › 执行',
      '规则',
      '策略',
    ]);
  });
});

describe('排列：分组、匹配与截断', () => {
  it('按动作、跳转分组；名字开头命中排前面，只在别名里命中的排后面', () => {
    const commands = buildCommands(paletteContext().context);
    const sections = arrangeCommands(commands, '执行');
    expect(sections.map((section) => section.group)).toEqual([
      'action',
      'navigate',
    ]);
    const labels = sections.map((section) =>
      section.entries.map((entry) => entry.command.label),
    );
    // 「打开执行候选」名字里含「执行」；「落地候选产物」只在说明里提到「协作 › 执行」。
    expect(labels[0].slice(0, 2)).toEqual(['打开执行候选', '落地候选产物']);
    expect(labels[1]).toEqual(['协作 › 执行']);
    const [first] = sections[0].entries;
    expect(first.ranges).toEqual([[2, 4]]);
  });

  it('空输入列出全部命令；每类实体最多列 8 条，超出给出带总数的「全部 N 个项目」', () => {
    const projects = manyProjects(10, [9]);
    const commands = buildCommands(paletteContext({ projects }).context);
    const [, navigate] = arrangeCommands(commands, '');
    const rows = navigate.entries.map((entry) => entry.command);
    const ids = rows.map((command) => command.id);
    // 每类 8 条之后紧跟它的「全部」行。
    const openRows = rows.filter((command) =>
      command.id.startsWith('project:'),
    );
    expect(openRows).toHaveLength(FAMILY_LIMIT);
    expect(ids[ids.indexOf(openRows.at(-1)!.id) + 1]).toBe('more:project');
    const all = byId(rows, 'more:project');
    expect(all.label).toBe('全部 10 个项目');
    expect(all.hint).toBe('这里列前 8 条');
    // 「全部」行清掉项目视图的筛选再打开，总数与行数对得上（T-117）。
    expect(effects(all, paletteContext().context)).toEqual({
      viewAllProjects: [[]],
    });
    expect(
      effects(byId(rows, 'more:source'), paletteContext().context),
    ).toEqual({ viewAllProjects: [[]] });
    const sourceRows = rows.filter((command) =>
      command.id.startsWith('source:'),
    );
    expect(sourceRows).toHaveLength(FAMILY_LIMIT);
    expect(ids[ids.indexOf(sourceRows.at(-1)!.id) + 1]).toBe('more:source');
    expect(byId(rows, 'more:source').label).toBe('全部 9 个项目来源');
    // 项目 id 恰好叫 all 也不会和「全部」行撞 key。
    expect(new Set(ids).size).toBe(ids.length);
    // 输入缩小范围后按匹配数说明，总数仍是全部项目数：12 个项目里「项目0」命中 00–09 共 10 个。
    const twelve = buildCommands(
      paletteContext({ projects: manyProjects(12) }).context,
    );
    const narrowed = arrangeCommands(twelve, '项目0')
      .flatMap((section) => section.entries)
      .map((entry) => entry.command);
    expect(
      narrowed.filter((command) => command.id.startsWith('project:p')),
    ).toHaveLength(FAMILY_LIMIT);
    const narrowedAll = narrowed.find(
      (command) => command.id === 'more:project',
    );
    expect(narrowedAll?.label).toBe('全部 12 个项目');
    expect(narrowedAll?.hint).toBe('匹配 10 条，这里列前 8 条');
    // 只剩一条命中：没有「全部」行，也没有空的动作组。
    const single = arrangeCommands(commands, '项目09');
    expect(single.map((section) => section.group)).toEqual(['navigate']);
    expect(single[0].entries.map((entry) => entry.command.id)).toEqual([
      'project:p09',
    ]);
  });

  it('不超过 8 条就不加「全部」行；黑板任务超出时说明汇总是否截断', () => {
    const commands = buildCommands(
      paletteContext({ projects: [project] }).context,
    );
    expect(
      arrangeCommands(commands, '')
        .flatMap((section) => section.entries)
        .some((entry) => entry.command.id.startsWith('more:')),
    ).toBe(false);
    const overview = parseBoardOverview(
      overviewResponse(
        Array.from({ length: 9 }, (_, index) => ({
          id: taskId(String(index + 1)),
          title: `任务 ${index + 1}`,
          project: '项目/星舟',
          status: 'queued' as const,
        })),
        { truncated: true },
      ),
      24,
    );
    const withTasks = buildCommands(paletteContext({ overview }).context);
    const rows = arrangeCommands(withTasks, '任务', {
      tasksTruncated: overview.itemsTruncated,
    })
      .flatMap((section) => section.entries)
      .map((entry) => entry.command);
    const all = rows.find((command) => command.id === 'more:task');
    expect(all?.label).toBe('全部黑板任务（汇总只取到 9 条）');
    expect(all?.hint).toBe('这里列前 8 条；去协作拓扑看全部');
    expect(effects(all!, paletteContext().context)).toEqual({
      goTo: [[{ view: 'collab' }]],
    });
  });

  it('多段输入每段都要命中，名字里的每段各标一条红线', () => {
    const commands = buildCommands(paletteContext().context);
    const [navigate] = arrangeCommands(commands, '协作 时间');
    expect(navigate.entries.map((entry) => entry.command.label)).toEqual([
      '协作 › 时间线',
    ]);
    expect(navigate.entries[0].ranges).toEqual([
      [0, 2],
      [5, 7],
    ]);
    expect(
      arrangeCommands(commands, 'POLICY')[1].entries[0].command.label,
    ).toBe('策略');
    expect(arrangeCommands(commands, '合并 PR')).toEqual([]);
  });
});
