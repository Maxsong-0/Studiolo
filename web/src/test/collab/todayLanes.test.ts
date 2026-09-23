import { describe, expect, it } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import type {
  BoardOverviewEvent,
  BoardOverviewItem,
} from '../../boardOverviewModel';
import { parseBoardOverview } from '../../boardOverviewModel';
import { OWNER_LANE, WORK_GAP_MS, todayLanes } from '../../collab/todayLanes';

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
): BoardOverviewEvent {
  return {
    at: new Date(day(hour)).toISOString(),
    project: '项目/Studiolo',
    taskId: ulid(n),
    action,
    phase: 'outcome',
    attemptId: action.startsWith('execution.') ? null : `A-${ulid(n)}`,
  };
}

describe('今天的时间线', () => {
  it('同一执行者间隔不超过 90 分钟的事件并成一段黑块，主人的 execution.* 单列成红块「你」', () => {
    const items = [running(1, 'root-A'), running(2, 'root-B')];
    const lanes = todayLanes(
      items,
      [
        event(1, -3), // 昨天：不画
        event(1, 8.5),
        event(1, 9.5),
        event(1, 10.1),
        event(2, 12.8),
        event(1, 11.7, 'execution.review'),
        event(1, 11.7 + 1 / 3600, 'execution.review'),
        event(1, 14),
      ],
      day(19.95),
    );
    expect(lanes).not.toBeNull();
    expect(lanes!.lanes).toEqual([
      {
        // 两个根实例前 4 位相同：标签一起加长到能区分为止
        name: 'Claude · root-A',
        blocks: [
          { start: day(8.5), end: day(10.1), tone: 'work' },
          { start: day(14), end: day(14), tone: 'work' },
        ],
      },
      {
        name: 'Claude · root-B',
        blocks: [{ start: day(12.8), end: day(12.8), tone: 'work' }],
      },
      {
        name: OWNER_LANE,
        blocks: [
          { start: day(11.7), end: day(11.7 + 1 / 3600), tone: 'owner' },
        ],
      },
    ]);
    expect(lanes!.events).toBe(7);
    expect(WORK_GAP_MS).toBe(90 * 60_000);
  });

  it('画幅至少 08:00–20:00，刻度首尾对齐、均分成不超过 5 段', () => {
    const lanes = todayLanes(
      [running(1, 'root-A')],
      [event(1, 9), event(1, 18)],
      day(19.5),
    );
    expect(lanes!.from).toBe(day(8));
    expect(lanes!.to).toBe(day(20));
    expect(lanes!.ticks).toEqual(['08:00', '11:00', '14:00', '17:00', '20:00']);
    const early = todayLanes(
      [running(1, 'root-A')],
      [event(1, 2.5), event(1, 21.2)],
      day(21.5),
    );
    expect(early!.from).toBe(day(2));
    expect(early!.ticks).toEqual([
      '02:00',
      '06:00',
      '10:00',
      '14:00',
      '18:00',
      '22:00',
    ]);
  });

  it('今天没有事件时返回 null，不画空泳道', () => {
    expect(
      todayLanes([running(1, 'root-A')], [event(1, -5)], day(10)),
    ).toBeNull();
  });

  it('假数据：四条执行线加「你」，M4 接续线两段工作', () => {
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
    const lanes = todayLanes(
      overview.items,
      overview.events,
      Date.parse(overview.generatedAt),
    )!;
    expect(lanes.lanes).toHaveLength(5);
    expect(lanes.lanes.at(-1)?.name).toBe(OWNER_LANE);
    expect(lanes.lanes.map((lane) => lane.blocks.length)).toEqual([
      // 组序按根实例 id：M4 接续线（R2）、全项目收尾线（R3）、接入线（R4）、重设计线（R1）；
      // Codex 根统筹（R5）今天没有事件，不列；「你」四次审批
      2,
      2, 1, 1, 4,
    ]);
    expect(lanes.ticks[0]).toBe('08:00');
  });
});
