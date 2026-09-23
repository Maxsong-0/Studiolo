import { describe, expect, it } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import type { OverviewState } from '../../cockpit/model';
import { MINI_TASKS, actionTitle, planPending } from '../../cockpit/pending';
import { fixture } from '../fixtures';
import { fixtureData } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const { inbox, projects } = data.workbench;
const item = (id: string) =>
  data.overview.items.find((entry) => entry.id === id)!;

describe('待我动作：统一编号与圈号', () => {
  it('黑板里等你的任务排前面（最久没动静的先列），工作台待办接着编号，统一 1..N', () => {
    const plan = planPending(ready, inbox, projects);
    expect(plan.entries.map((entry) => entry.n)).toEqual(
      Array.from({ length: 3 + inbox.length }, (_, index) => index + 1),
    );
    const board = plan.entries.flatMap((entry) =>
      entry.source === 'board' ? [entry] : [],
    );
    // T-093 最后进展 10:03，T-106 15:03，T-108 15:33：越早停下的越久没动静。
    expect(board.map((entry) => entry.item.id)).toEqual([
      TASK.T093,
      TASK.T106,
      TASK.T108,
    ]);
    expect(board.map((entry) => entry.kind)).toEqual([
      '待修订',
      '待验收',
      '待验收',
    ]);
    // 黑板条目的项目名取工作台里同一 root 的项目名。
    expect(board.map((entry) => entry.project)).toEqual([
      'Studiolo',
      'Studiolo',
      'Studiolo',
    ]);
    const tail = plan.entries.slice(board.length);
    expect(tail.every((entry) => entry.source === 'inbox')).toBe(true);
    expect(
      tail.map((entry) => (entry.source === 'inbox' ? entry.action.id : '')),
    ).toEqual(inbox.map((action) => action.id));
    // 类型取工作台待办的 kind 原文。
    expect(tail.map((entry) => entry.kind)).toEqual(
      inbox.map((action) => action.kind),
    );
    expect(plan.total).toBe(3 + inbox.length);
  });

  it('画进缩略图的黑板条目实心并生成同号标记；工作台待办一律空心，标记只来自黑板', () => {
    const plan = planPending(ready, inbox, projects);
    expect(plan.markers).toEqual([
      { n: 1, target: { taskId: TASK.T093 } },
      { n: 2, target: { taskId: TASK.T106 } },
      { n: 3, target: { taskId: TASK.T108 } },
    ]);
    for (const marker of plan.markers) {
      const entry = plan.entries[marker.n - 1];
      expect(entry.source).toBe('board');
      expect(entry.source === 'board' && entry.marked).toBe(true);
    }
    const drawn = new Set(plan.miniItems.map((entry) => entry.id));
    for (const marker of plan.markers)
      expect('taskId' in marker.target && drawn.has(marker.target.taskId)).toBe(
        true,
      );
    expect(
      plan.entries.filter((entry) => entry.source === 'inbox'),
    ).toHaveLength(inbox.length);
  });

  it('缩略图最多 6 条：等你的先上，再补尝试仍在进行的，其余算进「另 N 条」', () => {
    const plan = planPending(ready, inbox, projects);
    expect(MINI_TASKS).toBe(6);
    // 未结束 7 条（简报站 T-021 今天已结束，不画）。
    expect(plan.unfinished).toBe(7);
    expect(plan.miniItems.map((entry) => entry.id).sort()).toEqual(
      [TASK.T046, TASK.T093, TASK.T106, TASK.T108, TASK.T109, TASK.T130].sort(),
    );
    // 只剩已批准、写域未释放的 T-092 没上图。
    expect(plan.omitted).toBe(1);
    // 缩略图条目保持汇总原顺序（项目、id）。
    const order = data.overview.items.map((entry) => entry.id);
    const positions = plan.miniItems.map((entry) => order.indexOf(entry.id));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
  });

  it('等你的任务多过缩略图容量时，排在后面的退成空心圈、不生成标记', () => {
    const plan = planPending(ready, [], projects, 2);
    expect(
      plan.entries.map((entry) => entry.source === 'board' && entry.marked),
    ).toEqual([true, true, false]);
    expect(plan.markers.map((marker) => marker.n)).toEqual([1, 2]);
    expect(plan.miniItems).toHaveLength(2);
    expect(plan.omitted).toBe(5);
  });

  it('「最久」只算自带时间的黑板条目，以汇总生成时刻为准', () => {
    const plan = planPending(ready, inbox, projects);
    const now = Date.parse(data.overview.generatedAt);
    expect(plan.oldestWaitMs).toBe(
      now - item(TASK.T093).progress!.lastProgressMs,
    );
    // 工作台待办不带时间：只有它们时不给「最久」。
    expect(
      planPending({ kind: 'disabled' }, fixture.inbox, fixture.projects)
        .oldestWaitMs,
    ).toBeNull();
  });

  it('黑板未启用：等你的黑板任务确实为 0，总数就是工作台待办数；不画缩略图', () => {
    const plan = planPending({ kind: 'disabled' }, inbox, projects);
    expect(plan.total).toBe(inbox.length);
    expect(plan.entries.every((entry) => entry.source === 'inbox')).toBe(true);
    expect(plan.entries.map((entry) => entry.n)).toEqual(
      inbox.map((_, index) => index + 1),
    );
    expect(plan.markers).toEqual([]);
    expect(plan.miniItems).toEqual([]);
    expect(plan.unfinished).toBeNull();
  });

  it.each<OverviewState>([
    { kind: 'loading' },
    { kind: 'error', message: '黑板许可已满。' },
  ])(
    '黑板 $kind 时等你的黑板任务数不可知：总数为 null，列表只有工作台待办',
    (state) => {
      const plan = planPending(state, inbox, projects);
      expect(plan.total).toBeNull();
      expect(plan.entries).toHaveLength(inbox.length);
      expect(plan.markers).toEqual([]);
    },
  );

  it('汇总被截断时照实标出', () => {
    const plan = planPending(
      { kind: 'ready', overview: { ...data.overview, itemsTruncated: true } },
      inbox,
      projects,
    );
    expect(plan.truncated).toBe(true);
    expect(planPending(ready, inbox, projects).truncated).toBe(false);
  });

  it('待办标题以「类型：」开头时去掉重复的类型前缀', () => {
    const [action] = fixture.inbox;
    expect(actionTitle({ ...action, title: '待验收：验收只读工作台' })).toBe(
      '验收只读工作台',
    );
    expect(actionTitle(action)).toBe('验收只读工作台');
  });
});
