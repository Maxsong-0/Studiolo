import { describe, expect, it } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { TASK } from '../../../dev/fixtures/ids';
import type {
  BoardOverviewItem,
  BoardOverviewStatus,
} from '../../boardOverviewModel';
import { parseBoardOverview } from '../../boardOverviewModel';
import {
  executorLabels,
  executorOf,
  groupByExecutor,
} from '../../collab/executorGrouping';
import {
  COLUMNS,
  DORMANT_AFTER_MS,
  EXECUTOR_BUS_X,
  PATH_BUS_X,
  findOverlaps,
  fitText,
  isDormant,
  pathContains,
  topologyLayout,
  topologySummary,
} from '../../collab/topologyLayout';

const NOW = Date.UTC(2026, 8, 22, 12);
const HOUR = 3_600_000;
const ulid = (n: number) => `01M3${String(n).padStart(22, '0')}`;

/** 认领中的任务：根实例 root 派生出 holder。 */
function claimed(
  n: number,
  paths: string[] = [],
  options: {
    root?: string;
    status?: BoardOverviewStatus;
    lastProgress?: number | null;
    project?: string;
    title?: string;
  } = {},
): BoardOverviewItem {
  const root = options.root ?? 'root-A';
  const holder = `${root}-child-${n}`;
  const status = options.status ?? 'running';
  const active = [
    'claimed',
    'running',
    'cancel_requested',
    'committing',
  ].includes(status);
  const last =
    options.lastProgress === undefined ? NOW - 60_000 : options.lastProgress;
  return {
    id: ulid(n),
    title: options.title ?? `任务 ${n}`,
    project: options.project ?? '项目/Studiolo',
    status,
    parent: null,
    adapter: 'claude',
    holder,
    holderLineage: [
      { id: root, agentType: 'claude' },
      { id: holder, agentType: 'claude' },
    ],
    paths,
    attempt: {
      phase: active ? (status as 'running') : 'completed',
      worker: holder,
      active,
      leaseUntilMs: NOW + HOUR,
    },
    progress:
      last === null ? null : { tools: 3, edits: 1, lastProgressMs: last },
    receiptStatus: null,
  };
}

/** 从未认领：没有持有者、尝试与进度。 */
function unclaimed(
  n: number,
  paths: string[] = [],
  status: BoardOverviewStatus = 'queued',
): BoardOverviewItem {
  return {
    id: ulid(n),
    title: `任务 ${n}`,
    project: '项目/Studiolo',
    status,
    parent: null,
    adapter: 'codex',
    holder: null,
    holderLineage: [],
    paths,
    attempt: null,
    progress: null,
    receiptStatus: null,
  };
}

describe('写域重叠（与 control-db contains 同一规则）', () => {
  it('相等或在 / 边界上是前缀才算包含，比较前转小写', () => {
    expect(pathContains('web', 'web')).toBe(true);
    expect(pathContains('web', 'web/src/policyApi.ts')).toBe(true);
    expect(pathContains('Web/Src', 'web/src/policyApi.ts')).toBe(true);
    expect(pathContains('web', 'webapp')).toBe(false);
    expect(pathContains('web/src', 'web')).toBe(false);
    expect(pathContains('docs/m3', 'docs/m30')).toBe(false);
  });

  it('跨任务的相等、前缀与大小写不同都判重叠；非边界前缀与同一任务内的嵌套不算', () => {
    const overlaps = findOverlaps([
      claimed(1, ['web', 'docs/m3']),
      claimed(2, ['WEB/src/policyApi.ts', 'webapp']),
      claimed(3, ['docs/m3', 'docs/m3/server-agent.md']),
      claimed(4, ['docs/m30']),
    ]);
    expect(overlaps).toEqual([
      // 1 的 docs/m3 与 3 的 docs/m3 相等，也包含 3 的 docs/m3/server-agent.md
      {
        a: { taskId: ulid(1), path: 'docs/m3' },
        b: { taskId: ulid(3), path: 'docs/m3' },
      },
      {
        a: { taskId: ulid(1), path: 'docs/m3' },
        b: { taskId: ulid(3), path: 'docs/m3/server-agent.md' },
      },
      // 大小写不同的前缀
      {
        a: { taskId: ulid(1), path: 'web' },
        b: { taskId: ulid(2), path: 'WEB/src/policyApi.ts' },
      },
    ]);
  });

  it('只看未结束条目：已结束的不参与；休眠的照样参与', () => {
    const done = { ...claimed(2, ['web/src']), status: 'succeeded' as const };
    const sleeping = claimed(3, ['web/src/ui'], {
      lastProgress: NOW - 7 * HOUR,
    });
    const overlaps = findOverlaps([claimed(1, ['web']), done, sleeping]);
    expect(overlaps).toEqual([
      {
        a: { taskId: ulid(1), path: 'web' },
        b: { taskId: ulid(3), path: 'web/src/ui' },
      },
    ]);
  });

  it('假数据样例：T-130 的 web 与 T-092 的 web/src/policyApi.ts 判为重叠，T-046 判为休眠', () => {
    const now = new Date(2026, 8, 22, 20, 31).getTime();
    const api = createFixtureApi({ now: () => now });
    const response = api.handle({
      method: 'GET',
      url: '/api/v1/board/overview?window_hours=24',
      headers: {},
    });
    expect(response.status).toBe(200);
    const overview = parseBoardOverview(response.body, 24);
    const layout = topologyLayout(overview.items, {
      mode: 'full',
      now: Date.parse(overview.generatedAt),
    });
    expect(layout.overlaps).toEqual([
      {
        a: { taskId: TASK.T092, path: 'web/src/policyApi.ts' },
        b: { taskId: TASK.T130, path: 'web' },
      },
    ]);
    const red = layout.paths.filter((row) => row.overlaps.length > 0);
    expect(red.map((row) => row.path).sort()).toEqual([
      'web',
      'web/src/policyApi.ts',
    ]);
    expect(red.find((row) => row.path === 'web/src/policyApi.ts')?.note).toBe(
      '写域重叠：包含于 web（T-130 前端全面重设计 · 进行中）',
    );
    expect(layout.brackets).toHaveLength(1);
    expect(layout.dormant).toEqual([TASK.T046]);
    // 简报站那条已结束，只因窗口内有事件才出现在汇总里：拓扑不画
    expect(layout.tasks.map((task) => task.id)).not.toContain(TASK.B021);
    expect(
      topologySummary(overview.items, Date.parse(overview.generatedAt)),
    ).toEqual({
      running: 6,
      overlaps: 1,
      waiting: 3,
      dormant: 1,
    });
  });
});

describe('休眠判定', () => {
  it('认领三态且最近进度超过 6 小时算休眠，恰好 6 小时不算', () => {
    expect(
      isDormant(
        claimed(1, [], { lastProgress: NOW - DORMANT_AFTER_MS - 1 }),
        NOW,
      ),
    ).toBe(true);
    expect(
      isDormant(claimed(1, [], { lastProgress: NOW - DORMANT_AFTER_MS }), NOW),
    ).toBe(false);
    expect(
      isDormant(
        claimed(1, [], {
          status: 'claimed',
          lastProgress: NOW - 4 * 24 * HOUR,
        }),
        NOW,
      ),
    ).toBe(true);
  });

  it('从未认领也从未有进度的草稿与待认领算休眠；认领了但还没回报进度的不算', () => {
    expect(isDormant(unclaimed(1), NOW)).toBe(true);
    expect(isDormant(unclaimed(1, [], 'draft'), NOW)).toBe(true);
    expect(isDormant(claimed(1, [], { lastProgress: null }), NOW)).toBe(false);
  });

  it('等你（needs_*）与已结束的永远不算休眠，哪怕很久没动', () => {
    expect(isDormant(unclaimed(1, [], 'needs_reconfirm'), NOW)).toBe(false);
    expect(
      isDormant(
        claimed(1, [], {
          status: 'needs_review',
          lastProgress: NOW - 30 * HOUR,
        }),
        NOW,
      ),
    ).toBe(false);
    expect(
      isDormant(
        claimed(1, [], { status: 'succeeded', lastProgress: NOW - 30 * HOUR }),
        NOW,
      ),
    ).toBe(false);
    expect(
      isDormant(
        claimed(1, [], { status: 'approved', lastProgress: NOW - 30 * HOUR }),
        NOW,
      ),
    ).toBe(false);
  });
});

describe('执行者分组与排序', () => {
  it('按实例链根节点归组，组内按任务 id 升序；从未认领的按 adapter 归组、排在最后', () => {
    const items = [
      claimed(5, [], { root: 'root-B' }),
      unclaimed(4),
      claimed(3, [], { root: 'root-A' }),
      claimed(1, [], { root: 'root-B' }),
    ];
    const groups = groupByExecutor(items);
    expect(groups.map((group) => group.key)).toEqual([
      'instance:root-A',
      'instance:root-B',
      'adapter:codex',
    ]);
    expect(groups.map((group) => group.items.map((item) => item.id))).toEqual([
      [ulid(3)],
      [ulid(1), ulid(5)],
      [ulid(4)],
    ]);
    // 与输入顺序无关
    expect(groupByExecutor([...items].reverse())).toEqual(groups);
  });

  it('标签是「agent 名 · 根 id 前 4 位」；前 4 位撞了就一起加长，未认领的用 adapter 名', () => {
    const labels = executorLabels(
      [
        claimed(1, [], { root: '01M34CMPV0' }),
        claimed(2, [], { root: '01M31JGSP0' }),
        unclaimed(3),
      ].map(executorOf),
    );
    expect([...labels.values()]).toEqual([
      'Claude · 01M34',
      'Claude · 01M31',
      'Codex',
    ]);
    expect(
      executorLabels(
        [claimed(1, [], { root: '01M34CMPV0' })].map(executorOf),
      ).get('instance:01M34CMPV0'),
    ).toBe('Claude · 01M3');
  });
});

describe('几何', () => {
  const layoutOf = (items: BoardOverviewItem[], options = {}) =>
    topologyLayout(items, { mode: 'full', now: NOW, ...options });

  it('单条路径与任务同高；多条路径以任务行为中心上下对称', () => {
    const layout = layoutOf([claimed(1, ['a']), claimed(2, ['b', 'c', 'd'])]);
    const [first, second] = layout.tasks;
    const pathsOf = (id: string) =>
      layout.paths.filter((row) => row.taskId === id);
    expect(pathsOf(first.id)[0].centerY).toBe(first.centerY);
    const centers = pathsOf(second.id).map((row) => row.centerY);
    expect(centers).toHaveLength(3);
    expect((centers[0] + centers[2]) / 2).toBe(second.centerY);
    expect(centers[1]).toBe(second.centerY);
    expect(centers[1] - centers[0]).toBe(centers[2] - centers[1]);
    // 行与行不重叠：第二行的最上一条路径框在第一行下面
    expect(pathsOf(second.id)[0].y).toBeGreaterThan(first.y + first.height);
  });

  it('连线是正交折线：执行者右缘 → x=235 → 任务左缘；任务右缘 → x=600 → 路径左缘', () => {
    const layout = layoutOf([
      claimed(1, ['a', 'b'], { root: 'r' }),
      claimed(2, ['c'], { root: 'r' }),
    ]);
    const executor = layout.executors[0];
    const claims = layout.edges.filter((edge) => edge.kind === 'claim');
    expect(claims.map((edge) => edge.d)).toEqual(
      layout.tasks.map(
        (task) =>
          `M${COLUMNS.executor.x + COLUMNS.executor.width} ${executor.centerY} H${EXECUTOR_BUS_X} V${task.centerY} H${COLUMNS.task.x - 4}`,
      ),
    );
    expect(executor.centerY).toBe(
      (layout.tasks[0].centerY + layout.tasks[1].centerY) / 2,
    );
    const pathEdges = layout.edges.filter((edge) => edge.kind === 'path');
    expect(pathEdges.map((edge) => edge.d)).toEqual(
      layout.paths.map((row) => {
        const task = layout.tasks.find((entry) => entry.id === row.taskId)!;
        return `M${COLUMNS.task.x + COLUMNS.task.width} ${task.centerY} H${PATH_BUS_X} V${row.centerY} H${COLUMNS.path.x}`;
      }),
    );
    expect(layout.width).toBe(1030);
  });

  it('重叠路径框下留一行说明：整摞（框＋说明）以任务行为中心', () => {
    const layout = layoutOf([
      claimed(1, ['web']),
      claimed(2, ['web/src'], { root: 'root-B' }),
    ]);
    for (const task of layout.tasks) {
      const row = layout.paths.find((entry) => entry.taskId === task.id)!;
      expect(row.noteY).not.toBeNull();
      expect(row.centerY).toBeLessThan(task.centerY);
      expect(row.y + (row.height + 18) / 2).toBe(task.centerY);
    }
    expect(layout.brackets).toHaveLength(1);
  });

  it('全休眠的执行者排在最后；关掉「显示休眠」就不画，并计入 hidden', () => {
    const items = [
      claimed(1, ['x'], { root: 'root-A', lastProgress: NOW - 10 * HOUR }),
      claimed(2, ['y'], { root: 'root-B' }),
    ];
    const shown = layoutOf(items);
    expect(
      shown.executors.map((row) => [row.key, row.dormant, row.detail]),
    ).toEqual([
      ['instance:root-B', false, '1 条任务'],
      ['instance:root-A', true, '休眠 · 1 条'],
    ]);
    expect(shown.tasks.find((task) => task.id === ulid(1))?.status).toBe(
      '休眠',
    );
    expect(shown.edges.filter((edge) => edge.dormant)).toHaveLength(2);
    const hidden = layoutOf(items, { showDormant: false });
    expect(hidden.tasks.map((task) => task.id)).toEqual([ulid(2)]);
    expect(hidden.hidden).toBe(1);
    expect(hidden.dormant).toEqual([ulid(1)]);
  });

  it('只看冲突：只留卷进重叠的任务，以及它们卷进重叠的那几条路径', () => {
    const layout = layoutOf(
      [
        claimed(1, ['web', 'docs']),
        claimed(2, ['web/src'], { root: 'root-B' }),
        claimed(3, ['crates'], { root: 'root-C' }),
      ],
      { onlyConflicts: true },
    );
    expect(layout.tasks.map((task) => task.id)).toEqual([ulid(1), ulid(2)]);
    expect(layout.paths.map((row) => row.path)).toEqual(['web', 'web/src']);
    expect(layout.hidden).toBe(1);
  });

  it('mini 只取 maxTasks 条并报告截掉的条数；圈号指到的、等你的优先留下，仍按组序排', () => {
    const items = [
      claimed(1, [], { root: 'root-A' }),
      claimed(2, [], { root: 'root-A' }),
      claimed(3, [], { root: 'root-A' }),
      claimed(4, [], { root: 'root-B', status: 'needs_review' }),
      claimed(5, [], { root: 'root-B' }),
      claimed(6, ['scripts'], { root: 'root-C' }),
      unclaimed(7),
    ];
    const layout = topologyLayout(items, {
      mode: 'mini',
      now: NOW,
      maxTasks: 3,
      markers: [{ n: 1, target: { path: 'scripts' } }],
    });
    expect(layout.omitted).toBe(4);
    expect(layout.tasks.map((task) => task.id)).toEqual([
      ulid(1),
      ulid(4),
      ulid(6),
    ]);
    expect(layout.markers).toHaveLength(1);
    const full = topologyLayout(items, { mode: 'full', now: NOW, maxTasks: 3 });
    expect(full.tasks).toHaveLength(7);
    expect(full.omitted).toBe(0);
  });

  it('圈号标在任务框右上角或路径框右侧；定位不到的不画', () => {
    const layout = layoutOf([claimed(1, ['scripts'])], {
      markers: [
        { n: 2, target: { taskId: ulid(1) } },
        { n: 1, target: { path: 'SCRIPTS' } },
        { n: 3, target: { taskId: 'missing' } },
      ],
    });
    expect(
      layout.markers.map((marker) => [marker.n, marker.cx, marker.r]),
    ).toEqual([
      [2, COLUMNS.task.x + COLUMNS.task.width + 26, 11],
      [1, COLUMNS.path.x + COLUMNS.path.width + 26, 11],
    ]);
    expect(layout.markers[0].label).toBe('待办 2：任务「任务 1」');
    expect(layout.markers.every((marker) => marker.cx + marker.r <= 1030)).toBe(
      true,
    );
  });

  it('选中的任务标「已选中」、连线加粗；标题放不下时截断补省略号', () => {
    const long = claimed(1, ['a'], {
      title: '一个非常非常非常非常非常非常非常长的任务标题需要截断',
    });
    const layout = layoutOf([long], { selectedTaskId: ulid(1) });
    const task = layout.tasks[0];
    expect(task.selected).toBe(true);
    expect(task.caption).toBe('Studiolo · 已选中');
    expect(task.titleText.endsWith('…')).toBe(true);
    expect(layout.edges.every((edge) => edge.emphasis)).toBe(true);
    expect(
      fitText('crates/local-server/src/board/overview.rs', 120, 12, 'end'),
    ).toMatch(/^….*overview\.rs$/);
  });
});
