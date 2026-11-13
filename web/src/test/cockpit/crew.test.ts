import { describe, expect, it } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import type {
  BoardOverview,
  BoardOverviewItem,
} from '../../boardOverviewModel';
import { CREW_ROWS, CREW_SPAN_MS, crew } from '../../cockpit/crew';
import { statusLabels } from '../../collab/boardLabels';
import { executorOf, overviewLabels } from '../../collab/executorGrouping';
import { OWNER_KEY, todayLanes } from '../../collab/todayLanes';
import { NOW, fixtureData } from './cockpitFixtures';

const data = fixtureData();
const overview = data.overview;
const item = (id: string) => overview.items.find((entry) => entry.id === id)!;
const labels = overviewLabels(overview);
/** 某条任务的执行者在整份汇总里的显示名（协作页用的同一套）。 */
const executor = (id: string) => labels.get(executorOf(item(id)).key)!;
const progressed = (id: string) => item(id).progress!.lastProgressMs;

const board = crew(overview);
const row = (id: string) =>
  board.rows.find((entry) => entry.name === executor(id))!;

/** 只留下几条任务（和它们的事件）的汇总。 */
function only(
  ids: readonly string[],
  items?: BoardOverviewItem[],
): BoardOverview {
  return {
    ...overview,
    items: items ?? overview.items.filter((entry) => ids.includes(entry.id)),
    events: overview.events.filter((event) => ids.includes(event.taskId)),
  };
}

/** 换一个根实例：同一条任务算到另一个执行者名下。 */
function heldBy(
  source: BoardOverviewItem,
  root: string,
  id: string,
): BoardOverviewItem {
  return {
    ...source,
    id,
    holder: root,
    holderLineage: [{ id: root, agentType: 'claude' }],
  };
}

describe('谁在干活：范围与刻度', () => {
  it('画最近 24 小时，终点对齐到下一个整点；「现在」是汇总生成时刻', () => {
    expect(board.now).toBe(NOW);
    expect(board.to - board.from).toBe(CREW_SPAN_MS);
    // 假数据此刻 20:31：终点 21:00，起点前一天 21:00。
    expect(new Date(board.to).getHours()).toBe(21);
    expect(new Date(board.to).getMinutes()).toBe(0);
    expect(board.to).toBeGreaterThanOrEqual(NOW);
  });

  it('刻度每 6 小时一格，跨到前一天的第一格写「昨天」', () => {
    expect(board.ticks).toEqual([
      { at: 0, label: '昨天 21:00' },
      { at: 0.25, label: '03:00' },
      { at: 0.5, label: '09:00' },
      { at: 0.75, label: '15:00' },
    ]);
  });
});

describe('谁在干活：一个执行者一行', () => {
  it('停下等你的在最前（等得最久的先），其次在跑的（最近有进展的先），最后是别的', () => {
    expect(board.rows.map((entry) => entry.name)).toEqual(
      [TASK.T093, TASK.T106, TASK.T130, TASK.T109, TASK.T046].map(executor),
    );
    expect(board.rows.map((entry) => entry.state)).toEqual([
      'waiting',
      'waiting',
      'running',
      'running',
      'idle',
    ]);
    expect(board).toMatchObject({ hidden: 0, running: 2, waiting: 2 });
    // 同一个根实例派生的子实例算同一个执行者：八条任务归到五个执行者。
    expect(board.rows.map((entry) => entry.key)).toEqual(
      [TASK.T093, TASK.T106, TASK.T130, TASK.T109, TASK.T046].map(
        (id) => executorOf(item(id)).key,
      ),
    );
    expect(executor(TASK.T092)).toBe(executor(TASK.T093));
    expect(executor(TASK.T108)).toBe(executor(TASK.T106));
    expect(executor(TASK.B021)).toBe(executor(TASK.T109));
  });

  it('一行摆一条任务：有等你的摆等得最久的那条，其余未结束的算进「另 N 条」', () => {
    // M4 接续线：T-093 待修订（10:03 起）、T-092 已批准未释放。
    expect(row(TASK.T093)).toMatchObject({
      task: { id: TASK.T093, title: item(TASK.T093).title },
      more: 1,
      status: '等你 10 时 28 分',
    });
    // 接入线：T-106（15:03 起）与 T-108（15:33 起）都等你，摆早的那条。
    expect(row(TASK.T106)).toMatchObject({
      task: { id: TASK.T106, title: item(TASK.T106).title },
      more: 1,
      status: '等你 5 时 28 分',
    });
    // 在跑的写状态和最近一次进展距今多久；已结束的 T-021 不算进「另 N 条」。
    expect(row(TASK.T130)).toMatchObject({
      task: { id: TASK.T130, title: item(TASK.T130).title },
      more: 0,
      status: `${statusLabels.running} · 34 分前`,
    });
    expect(row(TASK.T109)).toMatchObject({
      task: { id: TASK.T109, title: item(TASK.T109).title },
      more: 0,
      status: `${statusLabels.running} · 2 时前`,
    });
    // T-046 认领着但四天没有进展：算休眠，不算在跑。
    expect(row(TASK.T046)).toMatchObject({
      task: { id: TASK.T046, title: item(TASK.T046).title },
      more: 0,
      status: '休眠 · 4 天前',
      state: 'idle',
    });
  });

  it('黑段与协作页「今天的时间线」同一个 todayLanes，只是范围换成这 24 小时', () => {
    const lanes = todayLanes(overview, {
      range: { from: board.from, to: board.to, ticks: [] },
      maxLanes: Infinity,
    }).lanes.filter((lane) => lane.key !== OWNER_KEY);
    expect(lanes.length).toBeGreaterThan(0);
    for (const lane of lanes) {
      const found = board.rows.find((entry) => entry.key === lane.key)!;
      expect(found.name).toBe(lane.name);
      expect(found.blocks.filter((block) => block.tone === 'work')).toEqual(
        lane.blocks,
      );
    }
    // T-046 的事件停在四天前：这 24 小时没有黑段。
    expect(row(TASK.T046).blocks).toEqual([]);
    expect(row(TASK.T046).label).toBe(
      `${executor(TASK.T046)}，最近 24 小时：没有干活的记录`,
    );
  });

  it('红段：名下每条等你的任务，从最后一次进展一直画到汇总生成那一刻', () => {
    const red = (id: string) =>
      row(id).blocks.filter((block) => block.tone === 'owner');
    expect(red(TASK.T093)).toEqual([
      { start: progressed(TASK.T093), end: NOW, tone: 'owner' },
    ]);
    expect(red(TASK.T106)).toEqual([
      { start: progressed(TASK.T106), end: NOW, tone: 'owner' },
      { start: progressed(TASK.T108), end: NOW, tone: 'owner' },
    ]);
    expect(red(TASK.T130)).toEqual([]);
    expect(red(TASK.T109)).toEqual([]);
    const work = row(TASK.T093).blocks.filter((block) => block.tone === 'work');
    expect(row(TASK.T093).label).toBe(
      `${executor(TASK.T093)}，最近 24 小时：干活 ${work.length} 段，现在停下等你`,
    );
    expect(row(TASK.T130).label).not.toContain('等你');
  });
});

describe('谁在干活：谁该列出来', () => {
  it('名下没有未结束任务、只是这 24 小时里干过活的，也列出来，摆它最近结束的那条', () => {
    const reduced = only([TASK.B021, TASK.T046]);
    const rows = crew(reduced).rows;
    // 显示名按给进来的这份汇总算（只剩两个执行者时前缀不用加长）。
    const names = overviewLabels(reduced);
    expect(rows.map((entry) => entry.name)).toEqual(
      [TASK.B021, TASK.T046].map((id) => names.get(executorOf(item(id)).key)!),
    );
    expect(rows[0]).toMatchObject({
      state: 'idle',
      task: { id: TASK.B021, title: item(TASK.B021).title },
      more: 0,
      status: `${statusLabels.succeeded} · 9 时前`,
    });
    expect(rows[0].blocks.length).toBeGreaterThan(0);
    expect(rows[0].blocks.every((block) => block.tone === 'work')).toBe(true);
  });

  it('一条已结束、这 24 小时又没有动静的任务不单独撑起一行', () => {
    const quiet: BoardOverview = { ...only([TASK.B021]), events: [] };
    expect(crew(quiet).rows).toEqual([]);
    expect(crew(quiet)).toMatchObject({ hidden: 0, running: 0, waiting: 0 });
  });

  it('从未被认领的任务没有执行者，不在这里', () => {
    const unclaimed: BoardOverviewItem = {
      ...item(TASK.T109),
      status: 'queued',
      holder: null,
      holderLineage: [],
      attempt: null,
      progress: null,
    };
    expect(crew(only([], [unclaimed])).rows).toEqual([]);
  });

  it('等你的任务没有进展记录：只写「等你」、不画红段，排在有时间的后面', () => {
    const silent = heldBy(
      { ...item(TASK.T106), progress: null },
      '01M39ZZZZZZZZZZZZZZZZZZZZ1',
      '01M39ZZZZZZZZZZZZZZZZZZZZA',
    );
    const rows = crew(only([TASK.T093], [item(TASK.T093), silent])).rows;
    expect(rows.map((entry) => entry.state)).toEqual(['waiting', 'waiting']);
    expect(rows[0].task?.id).toBe(TASK.T093);
    expect(rows[1]).toMatchObject({
      task: { id: silent.id },
      status: '等你',
      blocks: [],
    });
  });

  it('执行者多于上限时只列前几行并报出没列的个数；在跑和等你的个数按全部执行者算', () => {
    const many = Array.from({ length: CREW_ROWS + 2 }, (_, index) =>
      heldBy(
        item(TASK.T106),
        `01M39ZZZZZZZZZZZZZZZZZZZZ${index}`,
        `01M39ZZZZZZZZZZZZZZZZZZZA${index}`,
      ),
    );
    const crowded = crew(only([], many));
    expect(crowded.rows).toHaveLength(CREW_ROWS);
    expect(crowded.hidden).toBe(2);
    expect(crowded.waiting).toBe(CREW_ROWS + 2);
    expect(crowded.running).toBe(0);
    // 同时停下的按名字排，结果不随输入顺序变。
    const names = crowded.rows.map((entry) => entry.name);
    expect(names).toEqual([...names].sort());
    expect(
      crew(only([], [...many].reverse())).rows.map((entry) => entry.name),
    ).toEqual(names);
  });
});
