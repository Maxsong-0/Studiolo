import { describe, expect, it } from 'vitest';
import { clockAt, SECOND } from '../../../dev/fixtures/clock';
import { INSTANCE, TASK } from '../../../dev/fixtures/ids';
import type {
  BoardOverview,
  BoardOverviewItem,
} from '../../boardOverviewModel';
import { actionText, isOwnerAction } from '../../cockpit/actions';
import { executorLabels, executorOf } from '../../cockpit/executors';
import {
  MAX_WORK_GAP_MS,
  OWNER_LANE,
  overviewRange,
  recentActivity,
  timelineLanes,
  timelineRange,
} from '../../cockpit/timeline';
import { NOW, fixtureData } from './cockpitFixtures';

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
  const { lanes, hidden } = timelineLanes(data.overview, range);
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
    const later = timelineLanes(data.overview, afternoon);
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
    const names = timelineLanes(trimmed, range).lanes.map(
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
