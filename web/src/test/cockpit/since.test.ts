import { describe, expect, it } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import type {
  BoardOverview,
  BoardOverviewEvent,
} from '../../boardOverviewModel';
import type { SeenMark } from '../../cockpit/lastSeen';
import {
  DAY_MS,
  type OverviewState,
  type PolicyPendingState,
} from '../../cockpit/model';
import {
  sinceLastSeen,
  type Since,
  type SinceInputs,
  type SinceStat,
} from '../../cockpit/since';
import type { ListedDesired } from '../../policyModel';
import { NOW, fixtureData, policyReady } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const drafts = policyReady(data.desired);
const freshness = data.workbench.freshness!;
const committed = BigInt(freshness.committed_through!);
const [proposal] = data.desired;

/** 假数据当天（2026-09-22）的某个本地钟点。 */
const today = (hour: number, minute = 0) =>
  new Date(2026, 8, 22, hour, minute).getTime();
const mark = (at: number, behind = 5n): SeenMark => ({
  at,
  committed: String(committed - behind),
});

function since(overrides: Partial<SinceInputs> = {}): Since {
  return sinceLastSeen({
    baseline: mark(today(8)),
    board: ready,
    policy: drafts,
    freshness,
    now: NOW,
    ...overrides,
  });
}
const stat = (result: Since, key: SinceStat['key']) =>
  result.stats.find((entry) => entry.key === key)!;

function withEvents(
  events: BoardOverviewEvent[],
  overrides: Partial<BoardOverview> = {},
): OverviewState {
  return {
    kind: 'ready',
    overview: { ...data.overview, events, ...overrides },
  };
}
const created = (
  taskId: string,
  at: number,
  phase: BoardOverviewEvent['phase'] = 'outcome',
): BoardOverviewEvent => ({
  at: new Date(at).toISOString(),
  project: '项目/Studiolo',
  taskId,
  action: 'task.create',
  phase,
  attemptId: null,
});

describe('自上次查看以来：四个数', () => {
  it('有上次查看的记录时，四个数都从那一刻数起', () => {
    const result = since();
    expect(result).toMatchObject({
      known: true,
      from: today(8),
      to: NOW,
      caption: '08:00 起，12 时 31 分',
    });
    expect(result.stats.map((entry) => [entry.key, entry.unit])).toEqual([
      ['done', '个任务完成'],
      ['created', '个新建任务'],
      ['drafts', '份新提案和草案'],
      ['writes', '次写入'],
    ]);
    // 简报站 T-021 今天 10:57 验收通过：现在是「已完成」，起点之后还有事件。
    expect(stat(result, 'done')).toEqual({
      key: 'done',
      value: 1,
      atLeast: false,
      unit: '个任务完成',
      note: 'T-021',
    });
    expect(stat(result, 'created')).toMatchObject({
      value: 0,
      atLeast: false,
      note: '黑板上没有新建的',
    });
    // 09:12 交来的那份 AI 提案。
    expect(stat(result, 'drafts')).toEqual({
      key: 'drafts',
      value: 1,
      atLeast: false,
      unit: '份新提案和草案',
      note: '其中 1 份是 AI 提的',
    });
    expect(stat(result, 'writes')).toEqual({
      key: 'writes',
      value: 5,
      atLeast: false,
      unit: '次写入',
      note: '网页编辑和 AI 写回',
    });
  });

  it('起点之前发生的不算', () => {
    const result = since({ baseline: mark(today(11), 0n) });
    expect(result.caption).toBe('11:00 起，9 时 31 分');
    expect(stat(result, 'done')).toMatchObject({
      value: 0,
      note: '黑板上没有新完成的',
    });
    expect(stat(result, 'drafts')).toMatchObject({
      value: 0,
      note: '没有新提交的',
    });
    expect(stat(result, 'writes').value).toBe(0);
  });

  it('这台浏览器还没有记录：先按最近 24 小时算，并照实说明；写入没法比', () => {
    const result = since({ baseline: null });
    expect(result).toMatchObject({
      known: false,
      from: NOW - DAY_MS,
      to: NOW,
      caption: '这台浏览器还没有上次查看的记录，先按最近 24 小时算',
    });
    expect(stat(result, 'done')).toMatchObject({ value: 1, atLeast: false });
    expect(stat(result, 'drafts').value).toBe(1);
    expect(stat(result, 'writes')).toEqual({
      key: 'writes',
      value: null,
      atLeast: false,
      unit: '次写入',
      note: '',
      reason: '没有上次查看时的提交水位，没法比。',
    });
  });

  it('起点写成人话：当天只写钟点，前一天写「昨天」，更早写月日', () => {
    expect(
      since({ baseline: mark(new Date(2026, 8, 21, 22, 40).getTime()) })
        .caption,
    ).toBe('昨天 22:40 起，21 时 51 分');
    expect(since({ baseline: mark(NOW - 3 * DAY_MS) }).caption).toBe(
      '9月19日 20:31 起，3 天；黑板那两个数只含最近 24 小时',
    );
  });

  it('起点早于黑板汇总的 24 小时窗口：黑板那两个数实际可能更多，策略和写入不受影响', () => {
    const result = since({ baseline: mark(NOW - 3 * DAY_MS) });
    expect(stat(result, 'done')).toMatchObject({ value: 1, atLeast: true });
    expect(stat(result, 'created')).toMatchObject({ value: 0, atLeast: true });
    expect(stat(result, 'drafts').atLeast).toBe(false);
    expect(stat(result, 'writes').atLeast).toBe(false);
    // 昨天 22:40 还在窗口里：不带「可能更多」。
    const inside = since({
      baseline: mark(new Date(2026, 8, 21, 22, 40).getTime()),
    });
    expect(stat(inside, 'done').atLeast).toBe(false);
  });

  it('黑板事件被截断时，黑板那两个数同样标成实际可能更多', () => {
    const result = since({
      board: {
        kind: 'ready',
        overview: { ...data.overview, eventsTruncated: true },
      },
    });
    expect(stat(result, 'done').atLeast).toBe(true);
    expect(stat(result, 'created').atLeast).toBe(true);
    expect(result.caption).toBe('08:00 起，12 时 31 分');
  });

  it('记号比现在的读数还新：起点收到「现在」，数都是 0，不出负数', () => {
    const result = since({ baseline: mark(NOW + 5 * 60_000) });
    expect(result).toMatchObject({ from: NOW, to: NOW });
    expect(result.caption).toBe('20:31 起，不到 1 分');
    expect(stat(result, 'done').value).toBe(0);
    expect(stat(result, 'drafts').value).toBe(0);
  });
});

describe('自上次查看以来：新建任务与点名', () => {
  it('只数起点之后 task.create 的结果事件；点名最多三条，编号开头的只留编号', () => {
    const titled = (id: string, title: string) => ({
      ...data.overview.items[0],
      id,
      title,
    });
    const board = withEvents(
      [
        created('01M39ZZZZZZZZZZZZZZZZZZZA1', today(7)),
        created('01M39ZZZZZZZZZZZZZZZZZZZA1', today(9), 'intent'),
        created('01M39ZZZZZZZZZZZZZZZZZZZA1', today(9, 1)),
        created('01M39ZZZZZZZZZZZZZZZZZZZA2', today(10)),
        created('01M39ZZZZZZZZZZZZZZZZZZZA3', today(11)),
        created('01M39ZZZZZZZZZZZZZZZZZZZA4', today(12)),
      ],
      {
        items: [
          titled('01M39ZZZZZZZZZZZZZZZZZZZA1', 'T-201 备份演练'),
          titled('01M39ZZZZZZZZZZZZZZZZZZZA2', 'F3 副本不再靠本机'),
          titled('01M39ZZZZZZZZZZZZZZZZZZZA3', '没有编号的一条很长很长的标题'),
        ],
      },
    );
    expect(stat(since({ board }), 'created')).toEqual({
      key: 'created',
      value: 4,
      atLeast: false,
      unit: '个新建任务',
      note: 'T-201 · F3 副本不再靠本机 · 没有编号的一条很长很… 等 4 条',
    });
    // 三条以内全部点名；任务不在汇总条目里时写任务号。
    const few = withEvents(
      [
        created('01M39ZZZZZZZZZZZZZZZZZZZA1', today(9)),
        created('01M39ZZZZZZZZZZZZZZZZZZZA9', today(10)),
      ],
      { items: [titled('01M39ZZZZZZZZZZZZZZZZZZZA1', 'T-201 备份演练')] },
    );
    expect(stat(since({ board: few }), 'created').note).toBe(
      'T-201 · 01M39ZZZZZ…',
    );
  });

  it('任务完成：现在是「已完成」、起点之后还有事件的才算；失败、停止的不算', () => {
    const finished = (
      id: string,
      status: 'succeeded' | 'failed' | 'cancelled',
    ) => ({
      ...data.overview.items.find((entry) => entry.id === TASK.B021)!,
      id,
      title: `T-30${id.slice(-1)} 收尾`,
      status,
    });
    const event = (taskId: string, at: number): BoardOverviewEvent => ({
      at: new Date(at).toISOString(),
      project: '项目/Studiolo',
      taskId,
      action: 'execution.review',
      phase: 'outcome',
      attemptId: null,
    });
    const board = withEvents(
      [
        event('01M39ZZZZZZZZZZZZZZZZZZZB1', today(9)),
        event('01M39ZZZZZZZZZZZZZZZZZZZB2', today(9)),
        event('01M39ZZZZZZZZZZZZZZZZZZZB3', today(9)),
        event('01M39ZZZZZZZZZZZZZZZZZZZB4', today(7)),
      ],
      {
        items: [
          finished('01M39ZZZZZZZZZZZZZZZZZZZB1', 'succeeded'),
          finished('01M39ZZZZZZZZZZZZZZZZZZZB2', 'failed'),
          finished('01M39ZZZZZZZZZZZZZZZZZZZB3', 'cancelled'),
          finished('01M39ZZZZZZZZZZZZZZZZZZZB4', 'succeeded'),
        ],
      },
    );
    expect(stat(since({ board }), 'done')).toMatchObject({
      value: 1,
      note: 'T-301',
    });
  });
});

describe('自上次查看以来：新提案和草案', () => {
  const later = (overrides: Partial<ListedDesired>): ListedDesired => ({
    ...proposal,
    ...overrides,
  });

  it('数起点之后提交的，不论现在什么状态；说明里写其中几份是 AI 提的', () => {
    const own = later({
      id: '01M33ZA2G0HQ8K8HMTV4NH3KKR',
      submitter_kind: 'owner',
      agent_type: null,
      agent_instance: null,
      status: 'activated',
      submitted_at: new Date(today(13)).toISOString(),
    });
    const mixed = since({ policy: policyReady([proposal, own]) });
    expect(stat(mixed, 'drafts')).toMatchObject({
      value: 2,
      note: '其中 1 份是 AI 提的',
    });
    const mine = since({ policy: policyReady([own]) });
    expect(stat(mine, 'drafts')).toMatchObject({
      value: 1,
      note: '都是你自己交的',
    });
  });

  it.each<[string, PolicyPendingState, string]>([
    ['还在读', { kind: 'loading' }, '策略草案读取中。'],
    ['没接入', { kind: 'absent' }, '本入口没有接入策略平面。'],
    [
      '读失败',
      { kind: 'error', message: '策略平面响应格式无效，请重新核对。' },
      '策略草案读取失败：策略平面响应格式无效，请重新核对。',
    ],
  ])('策略%s时这个数拿不到，写明原因，不影响另外三个', (_, policy, reason) => {
    const result = since({ policy });
    expect(stat(result, 'drafts')).toEqual({
      key: 'drafts',
      value: null,
      atLeast: false,
      unit: '份新提案和草案',
      note: '',
      reason,
    });
    expect(stat(result, 'done').value).toBe(1);
    expect(stat(result, 'writes').value).toBe(5);
  });
});

describe('自上次查看以来：黑板没读到', () => {
  it.each<[string, OverviewState, string]>([
    ['还在读', { kind: 'loading' }, '黑板汇总读取中。'],
    ['未启用', { kind: 'disabled' }, '黑板未启用。'],
    [
      '读失败',
      { kind: 'error', message: '黑板许可已满。' },
      '黑板汇总读取失败：黑板许可已满。',
    ],
  ])('黑板%s时黑板那两个数拿不到；「现在」改用本机时钟', (_, board, reason) => {
    const result = since({ board, now: NOW + 60 * 60_000 });
    for (const key of ['done', 'created'] as const)
      expect(stat(result, key)).toMatchObject({
        value: null,
        atLeast: false,
        note: '',
        reason,
      });
    expect(result.to).toBe(NOW + 60 * 60_000);
    expect(result.caption).toBe('08:00 起，13 时 31 分');
    expect(stat(result, 'drafts').value).toBe(1);
    expect(stat(result, 'writes').value).toBe(5);
  });
});

describe('自上次查看以来：写入次数', () => {
  const writes = (overrides: Partial<SinceInputs>) =>
    stat(since(overrides), 'writes');

  it('提交水位是 u64 的十进制串：超过 2^53 也照样相减', () => {
    expect(
      writes({
        baseline: { at: today(8), committed: '18446744073709551000' },
        freshness: { ...freshness, committed_through: '18446744073709551615' },
      }).value,
    ).toBe(615);
  });

  it.each<[string, Partial<SinceInputs>, string]>([
    [
      '上次查看时快照没给水位',
      { baseline: { at: today(8), committed: null } },
      '上次查看时工作台快照没有给出提交水位。',
    ],
    [
      '这次快照没给新鲜度',
      { freshness: undefined },
      '工作台快照没有给出提交水位。',
    ],
    [
      '这次快照的水位是空的',
      { freshness: { ...freshness, committed_through: null } },
      '工作台快照没有给出提交水位。',
    ],
    [
      '水位比上次还小（换了库）',
      { baseline: { at: today(8), committed: String(committed + 1n) } },
      '提交水位比上次查看时还小，没法比。',
    ],
    [
      '差得太大',
      {
        baseline: { at: today(8), committed: '0' },
        freshness: { ...freshness, committed_through: '18446744073709551615' },
      },
      '提交水位差得太大，没法显示。',
    ],
  ])('%s：显示「—」并写原因，不编数', (_, overrides, reason) => {
    expect(writes(overrides)).toEqual({
      key: 'writes',
      value: null,
      atLeast: false,
      unit: '次写入',
      note: '',
      reason,
    });
  });
});
