/*
 * 「今天的时间线」唯一实现（collab/todayLanes.ts）的规则测试，由两份旧测试合并而来（T-130 C4）：
 * - 驾驶舱 B1 的时间线测试（原在驾驶舱测试目录，已删除）：断言全部原样保留，只把入口换成合并后的函数；
 * - 协作页 B2 的本文件旧版：与定案冲突的（90 分钟、全部事件、固定 08–20、
 *   返回 null、按组序排泳道、你的泳道按 5 分钟合并）改成定案值并在测试名里写明，其余原样保留。
 */
import { describe, expect, it } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { clockAt, SECOND } from '../../../dev/fixtures/clock';
import { INSTANCE, TASK } from '../../../dev/fixtures/ids';
import type {
  BoardOverview,
  BoardOverviewEvent,
  BoardOverviewItem,
} from '../../boardOverviewModel';
import { parseBoardOverview } from '../../boardOverviewModel';
import {
  WORK_ACTIONS,
  actionText,
  isOwnerAction,
} from '../../collab/boardActions';
import { executorLabels, executorOf } from '../../collab/executorGrouping';
import {
  MAX_AI_LANES,
  MAX_WORK_GAP_MS,
  OWNER_KEY,
  OWNER_LANE,
  overviewRange,
  recentActivity,
  timelineRange,
  todayLanes,
} from '../../collab/todayLanes';
import { NOW, fixtureData } from '../cockpit/cockpitFixtures';

/* ---------- 驾驶舱 B1 的断言（原驾驶舱时间线测试） ---------- */

const data = fixtureData();
/** 假数据的钟点（8.5 = 08:30）换成当天的毫秒，与事件生成用同一个公式。 */
const at = (hour: number) => clockAt(NOW, hour);
const local = (hour: number, minute = 0) =>
  new Date(2026, 8, 22, hour, minute).getTime();

describe('时间范围', () => {
  it('终点是汇总时刻向上取整到整点；今天的活动早于最近 12 小时，就从今天第一条所在整点画起', () => {
    const range = overviewRange(data.overview);
    expect(range.to).toBe(local(21));
    // 最近 12 小时是 09:00 起，但今天最早的 08:30（M4 接续线认领 T-093）更早：从 08:00 画起。
    expect(range.from).toBe(local(8));
    // 13 小时分不成整点的 4、3、5、6、2 段，退回均分 4 段。
    expect(range.ticks).toEqual(['08:00', '11:15', '14:30', '17:45', '21:00']);
    // 不给范围时 todayLanes 就画这一段。
    expect(todayLanes(data.overview).range).toEqual(range);
  });

  it('刻度段数尽量让每个刻度落在整点', () => {
    expect(timelineRange(local(20), [local(8, 30)]).ticks).toEqual([
      '08:00',
      '11:00',
      '14:00',
      '17:00',
      '20:00',
    ]);
    expect(timelineRange(local(23, 10), [local(9, 5)]).ticks).toEqual([
      '09:00',
      '14:00',
      '19:00',
      '00:00',
    ]);
  });

  it('至少画最近 12 小时：刚过 0 点照样看得到前一晚的活动，换了日期也不空着', () => {
    const noon = timelineRange(local(14, 20));
    expect([noon.from, noon.to]).toEqual([local(3), local(15)]);
    // 00:07：最近 12 小时是昨天 13:00 到今天 01:00，昨晚 23:00 的活动在里面。
    const night = timelineRange(local(0, 7), [local(-1)]);
    expect([night.from, night.to]).toEqual([local(-11), local(1)]);
    const midnight = timelineRange(local(0, 0));
    expect([midnight.from, midnight.to]).toEqual([local(-12), local(0)]);
    // 昨天更早的活动只算「今天」以外，不把起点拉到 12 小时之前。
    const late = timelineRange(local(1, 30), [local(-20)]);
    expect(late.from).toBe(local(-10));
  });
});

describe('执行者', () => {
  const lane = (key: keyof typeof INSTANCE) => INSTANCE[key].slice(0, 5);

  it('按持有者实例链的根分组；前 4 位撞了就把同类型的前缀一起加长到互不相同', () => {
    const labels = executorLabels(data.overview.items.map(executorOf));
    const labelOf = (id: string) =>
      labels.get(
        executorOf(data.overview.items.find((item) => item.id === id)!).key,
      );
    // R1（重设计线）与 R4（接入线）的根 id 前 4 位都是 01M3：分成两条，前缀加长到 5 位。
    expect(labelOf(TASK.T130)).toBe(`Claude · ${lane('R1')}`);
    expect(labelOf(TASK.T106)).toBe(`Claude · ${lane('R4')}`);
    expect(labelOf(TASK.T108)).toBe(labelOf(TASK.T106));
    expect(labelOf(TASK.T109)).toBe(`Claude · ${lane('R3')}`);
    expect(labelOf(TASK.T093)).toBe(`Codex · ${lane('R2')}`);
    expect(labelOf(TASK.T046)).toBe(`Codex · ${lane('R5')}`);
    expect(new Set(labels.values()).size).toBe(labels.size);
  });

  it('前 4 位不撞时就用 4 位；没有持有者时用任务的 adapter', () => {
    const base = data.overview.items.find((item) => item.id === TASK.T130)!;
    const holderless: BoardOverviewItem = {
      ...base,
      status: 'queued',
      holder: null,
      holderLineage: [],
      attempt: null,
      progress: null,
    };
    const labels = executorLabels([executorOf(base), executorOf(holderless)]);
    expect([...labels.values()]).toEqual([
      `Claude · ${INSTANCE.R1.slice(0, 4)}`,
      'Claude',
    ]);
  });
});

describe('泳道', () => {
  const range = overviewRange(data.overview);
  const { lanes, hidden } = todayLanes(data.overview, { range });
  const blocks = (name: string) =>
    lanes
      .find((entry) => entry.name === name)!
      .blocks.map(({ start, end, tone }) => ({ start, end, tone }));
  const work = (start: number, end: number) => ({ start, end, tone: 'work' });

  it('一位执行者一条泳道，按显示名排好；你的决定单独一条排在最后', () => {
    expect(lanes.map((entry) => entry.name)).toEqual([
      `Claude · ${INSTANCE.R3.slice(0, 5)}`,
      `Claude · ${INSTANCE.R4.slice(0, 5)}`,
      `Claude · ${INSTANCE.R1.slice(0, 5)}`,
      `Codex · ${INSTANCE.R2.slice(0, 5)}`,
      OWNER_LANE,
    ]);
    // Codex 根统筹（T-046）四天前之后没有动静：今天没有泳道。
    expect(hidden).toBe(0);
  });

  it('泳道的键是执行者分组键（按根实例完整 id），「你」的键固定', () => {
    expect(lanes.map((entry) => entry.key)).toEqual([
      `instance:${INSTANCE.R3}`,
      `instance:${INSTANCE.R4}`,
      `instance:${INSTANCE.R1}`,
      `instance:${INSTANCE.R2}`,
      OWNER_KEY,
    ]);
  });

  it('同一任务相邻活动事件之间算一段工作；相隔超过 2 小时断开；同一执行者的段合并', () => {
    expect(MAX_WORK_GAP_MS).toBe(2 * 3_600_000);
    // 全项目收尾线：T-109 在 11:24 到 16:12 之间停了 4.8 小时，断成两段；
    // 同一条线上的简报站验收（10:18–10:51）落在第一段里，并进去。
    expect(blocks(`Claude · ${INSTANCE.R3.slice(0, 5)}`)).toEqual([
      work(at(9.0), at(11.4)),
      work(at(16.2), at(18.5)),
    ]);
    // 接入线：T-106（12:48–15:06）与 T-108（13:03–15:36）重叠，合成一段。
    expect(blocks(`Claude · ${INSTANCE.R4.slice(0, 5)}`)).toEqual([
      work(at(12.8), at(15.6) + SECOND),
    ]);
    // 重设计线：19:31 认领 T-130 起到 19:57 的工具调用。
    expect(blocks(`Claude · ${INSTANCE.R1.slice(0, 5)}`)).toEqual([
      work(at(19.52), at(19.95)),
    ]);
    // M4 接续线：T-093 上午一段，T-092 下午一段。
    expect(blocks(`Codex · ${INSTANCE.R2.slice(0, 5)}`)).toEqual([
      work(at(8.5), at(10.1) + SECOND),
      work(at(14.0), at(14.25) + SECOND),
    ]);
  });

  it('你的审批：execution.* 的 intent 到 outcome 画成一块，不混进执行者的工作段', () => {
    const owner = (hour: number) => ({
      start: at(hour),
      end: at(hour) + SECOND,
      tone: 'owner',
    });
    expect(blocks(OWNER_LANE)).toEqual([
      owner(10.95),
      owner(11.7),
      owner(13.0),
      owner(14.5),
    ]);
  });

  it('范围之外的段不画、没有段的执行者不占泳道；你的泳道空着也留着', () => {
    const afternoon = { from: local(16), to: local(20), ticks: [] };
    const later = todayLanes(data.overview, { range: afternoon });
    expect(later.lanes.map((entry) => entry.name)).toEqual([
      `Claude · ${INSTANCE.R3.slice(0, 5)}`,
      `Claude · ${INSTANCE.R1.slice(0, 5)}`,
      OWNER_LANE,
    ]);
    expect(later.lanes.at(-1)!.blocks).toEqual([]);
  });

  it('汇总被截断、事件的任务不在 items 里时，这些事件无从归属，不画', () => {
    const trimmed: BoardOverview = {
      ...data.overview,
      itemsTruncated: true,
      items: data.overview.items.filter((item) => item.id !== TASK.T130),
    };
    const names = todayLanes(trimmed, { range }).lanes.map(
      (entry) => entry.name,
    );
    expect(names).not.toContain(`Claude · ${INSTANCE.R1.slice(0, 5)}`);
  });
});

describe('最近动态', () => {
  const range = overviewRange(data.overview);

  it('取范围内的 outcome，去掉模型、工具、心跳这类高频遥测，新的在前', () => {
    const feed = recentActivity(data.overview, range);
    expect(
      feed.map(({ time, who, text, raw }) => ({ time, who, text, raw })),
    ).toEqual([
      {
        time: '19:33',
        who: `Claude · ${INSTANCE.R1.slice(0, 5)}`,
        text: '执行体启动 T-130 前端全面重设计',
        raw: 'runner.start（outcome）',
      },
      {
        time: '19:31',
        who: `Claude · ${INSTANCE.R1.slice(0, 5)}`,
        text: '认领 T-130 前端全面重设计',
        raw: 'task.claim（outcome）',
      },
      {
        time: '15:36',
        who: `Claude · ${INSTANCE.R4.slice(0, 5)}`,
        text: '提交候选 T-108 副本扩容纳入 Semiment-Docs',
        raw: 'task.runner_candidates（outcome）',
      },
    ]);
  });

  it('你的决定记在「你」名下', () => {
    const feed = recentActivity(data.overview, range, 20);
    expect(feed).toContainEqual(
      expect.objectContaining({
        time: '14:30',
        who: OWNER_LANE,
        text: '审阅 T-092 人侧 M4 L3 部署',
        raw: 'execution.review（outcome）',
      }),
    );
  });

  it('动作名只翻译后端真实存在的，没收录的原样显示', () => {
    expect(actionText('task.claim')).toBe('认领');
    expect(actionText('execution.land')).toBe('落地候选');
    expect(actionText('task.brand_new')).toBe('task.brand_new');
    expect(actionText('constructor')).toBe('constructor');
    // 你的决定只认执行接口记下的 execution.*。
    expect(isOwnerAction('execution.review')).toBe(true);
    expect(isOwnerAction('task.candidates_reviewed')).toBe(false);
  });
});

/* ---------- 协作页 B2 的断言（原 test/collab/todayLanes.test.ts） ---------- */

/** 本地时间：泳道按浏览器所在时区的「今天」画。 */
const day = (hour: number) =>
  new Date(2026, 8, 22).getTime() + Math.round(hour * 3_600_000);
const ulid = (n: number) => `01M3${String(n).padStart(22, '0')}`;

function running(n: number, root: string): BoardOverviewItem {
  const holder = `${root}-child`;
  return {
    id: ulid(n),
    title: `任务 ${n}`,
    project: '项目/Studiolo',
    status: 'running',
    parent: null,
    adapter: 'claude',
    holder,
    holderLineage: [
      { id: root, agentType: 'claude' },
      { id: holder, agentType: 'claude' },
    ],
    paths: [],
    attempt: {
      phase: 'running',
      worker: holder,
      active: true,
      leaseUntilMs: day(21),
    },
    progress: { tools: 1, edits: 0, lastProgressMs: day(19) },
    receiptStatus: null,
  };
}
function event(
  n: number,
  hour: number,
  action = 'task.heartbeat',
  phase: BoardOverviewEvent['phase'] = 'outcome',
): BoardOverviewEvent {
  return {
    at: new Date(day(hour)).toISOString(),
    project: '项目/Studiolo',
    taskId: ulid(n),
    action,
    phase,
    attemptId: action.startsWith('execution.') ? null : `A-${ulid(n)}`,
  };
}
/** 把条目与事件装成一份汇总（todayLanes 的入参）；生成时刻即窗口终点。 */
function overviewOf(
  items: BoardOverviewItem[],
  events: BoardOverviewEvent[],
  generatedAt: number,
): BoardOverview {
  const to = new Date(generatedAt).toISOString();
  return {
    generatedAt: to,
    windowFrom: new Date(generatedAt - 24 * 3_600_000).toISOString(),
    windowTo: to,
    policyHash: '0'.repeat(64),
    items,
    itemsTruncated: false,
    events,
    eventsTruncated: false,
  };
}

describe('今天的时间线', () => {
  it('同一任务相邻活动相隔不超过 2 小时并成一段黑块，主人的 execution.* 单列成红块「你」（定案：由「同一执行者全部事件、90 分钟」改为「只认执行体活动、同一任务 2 小时」；你的泳道按 intent→outcome 成块，两条都是 outcome 就各画一块，不再按 5 分钟合并）', () => {
    const items = [running(1, 'root-A'), running(2, 'root-B')];
    const lanes = todayLanes(
      overviewOf(
        items,
        [
          event(1, -3), // 昨天 21:00：落在最近 12 小时（08:00–20:00）之外，不画
          event(1, 8.5),
          event(1, 9.5),
          event(1, 10.1),
          event(2, 12.8),
          event(1, 11.7, 'execution.review'),
          event(1, 11.7 + 1 / 3600, 'execution.review'),
          event(1, 14),
        ],
        day(19.95),
      ),
    );
    expect(lanes.lanes).toEqual([
      {
        // 两个根实例前 4 位相同：标签一起加长到能区分为止
        key: 'instance:root-A',
        name: 'Claude · root-A',
        blocks: [
          { start: day(8.5), end: day(10.1), tone: 'work' },
          { start: day(14), end: day(14), tone: 'work' },
        ],
      },
      {
        key: 'instance:root-B',
        name: 'Claude · root-B',
        blocks: [{ start: day(12.8), end: day(12.8), tone: 'work' }],
      },
      {
        key: OWNER_KEY,
        name: OWNER_LANE,
        blocks: [
          { start: day(11.7), end: day(11.7), tone: 'owner' },
          {
            start: day(11.7 + 1 / 3600),
            end: day(11.7 + 1 / 3600),
            tone: 'owner',
          },
        ],
      },
    ]);
    expect(lanes.events).toBe(7);
    expect(MAX_WORK_GAP_MS).toBe(2 * 3_600_000);
  });

  it('画幅（定案：至少最近 12 小时、终点是汇总时刻向上取整到整点，今天更早有活动就从那个整点画起，不再固定 08:00–20:00）：刻度首尾对齐，段数按 4、3、5、6、2 挑能落在整点的', () => {
    const lanes = todayLanes(
      overviewOf(
        [running(1, 'root-A')],
        [event(1, 9), event(1, 18)],
        day(19.5),
      ),
    );
    expect(lanes.range.from).toBe(day(8));
    expect(lanes.range.to).toBe(day(20));
    expect(lanes.range.ticks).toEqual([
      '08:00',
      '11:00',
      '14:00',
      '17:00',
      '20:00',
    ]);
    const early = todayLanes(
      overviewOf(
        [running(1, 'root-A')],
        [event(1, 2.5), event(1, 21.2)],
        day(21.5),
      ),
    );
    expect(early.range.from).toBe(day(2));
    // 定案：02:00–22:00 共 20 小时，4 段正好落在整点（B2 原为每 4 小时一格共 6 个刻度）。
    expect(early.range.ticks).toEqual([
      '02:00',
      '07:00',
      '12:00',
      '17:00',
      '22:00',
    ]);
  });

  it('最近 12 小时里没有可画的事件时，不画执行者泳道，只留空着的「你」（定案：不再返回 null；B2 原为「今天没有事件时返回 null，不画空泳道」）', () => {
    const lanes = todayLanes(
      overviewOf([running(1, 'root-A')], [event(1, -5)], day(10)),
    );
    // 10:00 整点不再上取整：画昨天 22:00 到今天 10:00，昨天 19:00 那条落在外面。
    expect(lanes.range).toEqual({
      from: day(-2),
      to: day(10),
      ticks: ['22:00', '01:00', '04:00', '07:00', '10:00'],
    });
    expect(lanes.lanes).toEqual([
      { key: OWNER_KEY, name: OWNER_LANE, blocks: [] },
    ]);
    expect(lanes.events).toBe(0);
    expect(lanes.hidden).toBe(0);
  });

  it('假数据：四条执行线加「你」，M4 接续线两段工作（定案：泳道按显示名排序，不再按根实例 id 的组序）', () => {
    const now = new Date(2026, 8, 22, 20, 31).getTime();
    const api = createFixtureApi({ now: () => now });
    const overview = parseBoardOverview(
      api.handle({
        method: 'GET',
        url: '/api/v1/board/overview?window_hours=24',
        headers: {},
      }).body,
      24,
    );
    const lanes = todayLanes(overview);
    expect(lanes.lanes).toHaveLength(5);
    expect(lanes.lanes.at(-1)?.name).toBe(OWNER_LANE);
    expect(lanes.lanes.map((lane) => lane.blocks.length)).toEqual([
      // 按显示名：全项目收尾线（Claude · 01M2Y）、接入线（Claude · 01M31）、重设计线（Claude · 01M34）、
      // M4 接续线（Codex · 01M2J）；Codex 根统筹（R5）今天没有事件，不列；「你」四次审批
      2,
      1, 1, 2, 4,
    ]);
    expect(lanes.range.ticks[0]).toBe('08:00');
  });
});

/* ---------- 定案规则的补充断言 ---------- */

describe('定案规则', () => {
  it('只认执行体自己的活动（WORK_ACTIONS）：创建、对账、请求取消、黑板请求都不是执行体在干活，不画（B2 原为全部事件都画）', () => {
    const others = [
      'task.create',
      'task.reconcile',
      'task.cancel_requested',
      'board.request',
    ];
    for (const action of others) expect(WORK_ACTIONS.has(action)).toBe(false);
    const lanes = todayLanes(
      overviewOf(
        [running(1, 'root-A')],
        others.map((action, index) => event(1, 9 + index, action)),
        day(19.5),
      ),
    );
    expect(lanes.lanes).toEqual([
      { key: OWNER_KEY, name: OWNER_LANE, blocks: [] },
    ]);
    expect(lanes.events).toBe(0);
  });

  it('你的审批：同一任务同一动作的 intent 到 outcome 画成一块；只有 intent、还没有结果的画成瞬时块', () => {
    const lanes = todayLanes(
      overviewOf(
        [running(1, 'root-A')],
        [
          event(1, 11.7, 'execution.review', 'intent'),
          event(1, 11.7 + 1 / 3600, 'execution.review'),
          event(1, 15, 'execution.cancel', 'intent'),
        ],
        day(19.5),
      ),
    );
    expect(lanes.lanes).toEqual([
      {
        key: OWNER_KEY,
        name: OWNER_LANE,
        blocks: [
          { start: day(11.7), end: day(11.7 + 1 / 3600), tone: 'owner' },
          { start: day(15), end: day(15), tone: 'owner' },
        ],
      },
    ]);
    expect(lanes.events).toBe(3);
  });

  it(`AI 泳道最多 ${MAX_AI_LANES} 条：多出的按最近活动取舍并报出条数，留下的仍按显示名排`, () => {
    const roots = Array.from({ length: 8 }, (_, index) => `root-${index + 1}`);
    const lanes = todayLanes(
      overviewOf(
        roots.map((root, index) => running(index + 1, root)),
        // root-1 09:00 … root-8 16:00：root-1、root-2 的活动最早，被挤掉。
        roots.map((_, index) => event(index + 1, 9 + index)),
        day(19.5),
      ),
    );
    expect(lanes.hidden).toBe(2);
    expect(lanes.lanes.map((lane) => lane.name)).toEqual([
      'Claude · root-3',
      'Claude · root-4',
      'Claude · root-5',
      'Claude · root-6',
      'Claude · root-7',
      'Claude · root-8',
      OWNER_LANE,
    ]);
  });

  it('执行者名缺省按这份汇总自己的条目算；给了名字表（整份汇总的）就用它', () => {
    const scoped = overviewOf([running(1, 'root-A')], [event(1, 9)], day(19.5));
    // 只有一个根实例：前 4 位就够。
    expect(todayLanes(scoped).lanes[0]).toMatchObject({
      key: 'instance:root-A',
      name: 'Claude · root',
    });
    // 整份汇总里它与 root-B 撞了前缀，名字表给的是加长后的名字：泳道跟着用，不自己重算。
    const labels = executorLabels(
      [running(1, 'root-A'), running(2, 'root-B')].map(executorOf),
    );
    expect(todayLanes(scoped, { labels }).lanes[0]).toMatchObject({
      key: 'instance:root-A',
      name: 'Claude · root-A',
    });
  });
});
