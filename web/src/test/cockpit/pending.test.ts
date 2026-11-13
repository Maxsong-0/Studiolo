import { describe, expect, it } from 'vitest';
import { DESIRED_ID, TASK } from '../../../dev/fixtures/ids';
import type { OverviewState, PolicyPendingState } from '../../cockpit/model';
import {
  KIND_DRAFT,
  KIND_PROPOSAL,
  actionTitle,
  entryKey,
  pendingCount,
  planPending,
  waitingDesired,
  type PendingSources,
} from '../../cockpit/pending';
import type { ListedDesired } from '../../policyModel';
import { fixture } from '../fixtures';
import { NOW, fixtureData, policyReady } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const drafts = policyReady(data.desired);
const { inbox, projects } = data.workbench;
const [proposal] = data.desired;
const item = (id: string) =>
  data.overview.items.find((entry) => entry.id === id)!;

function plan(overrides: Partial<PendingSources> = {}) {
  return planPending({
    board: ready,
    policy: drafts,
    inbox,
    projects,
    ...overrides,
  });
}

/** 在假数据那份提案上改几个字段，造出别的草案。 */
function draft(overrides: Partial<ListedDesired>): ListedDesired {
  return { ...proposal, ...overrides };
}

const BOARD_FAILED: OverviewState = {
  kind: 'error',
  message: '黑板许可已满。',
};
const POLICY_FAILED: PolicyPendingState = {
  kind: 'error',
  message: '策略平面响应格式无效，请重新核对。',
};

describe('「等你」的队列：三路并成一列', () => {
  it('带时间的排前面、等得最久的在最上，工作台待办接在后面，统一编号 1..N', () => {
    const { entries, total } = plan();
    expect(entries.map((entry) => entry.n)).toEqual(
      Array.from({ length: 4 + inbox.length }, (_, index) => index + 1),
    );
    // 提案 09:12 提交；T-093 最后进展 10:03，T-106 15:03，T-108 15:33：越早开始等的越靠前，不分来源。
    expect(entries.slice(0, 4).map(entryKey)).toEqual([
      `policy:${DESIRED_ID}`,
      `board:${TASK.T093}`,
      `board:${TASK.T106}`,
      `board:${TASK.T108}`,
    ]);
    expect(entries.slice(0, 4).map((entry) => entry.kind)).toEqual([
      KIND_PROPOSAL,
      '待修订',
      '待验收',
      '待验收',
    ]);
    // 黑板条目的项目名取工作台里同一 root 的项目名。
    expect(
      entries.flatMap((entry) =>
        entry.source === 'board' ? [entry.project] : [],
      ),
    ).toEqual(['Studiolo', 'Studiolo', 'Studiolo']);
    // 工作台待办不带时间：保持服务端顺序，类型取 kind 原文。
    const tail = entries.slice(4);
    expect(tail.map(entryKey)).toEqual(
      inbox.map((action) => `inbox:${action.id}`),
    );
    expect(tail.map((entry) => entry.kind)).toEqual(
      inbox.map((action) => action.kind),
    );
    expect(tail.every((entry) => entry.waitMs === null)).toBe(true);
    expect(total).toBe(4 + inbox.length);
  });

  it('等了多久：黑板任务从最后一次进展算到汇总生成那一刻，草案从提交算到读到列表那一刻', () => {
    const { entries, oldestWaitMs } = plan();
    const generated = Date.parse(data.overview.generatedAt);
    const waits = new Map(
      entries.map((entry) => [entryKey(entry), entry.waitMs]),
    );
    for (const id of [TASK.T093, TASK.T106, TASK.T108])
      expect(waits.get(`board:${id}`)).toBe(
        generated - item(id).progress!.lastProgressMs,
      );
    const submitted = Date.parse(proposal.submitted_at);
    expect(waits.get(`policy:${DESIRED_ID}`)).toBe(NOW - submitted);
    // 「最久」是带时间的条目里最大的那个：这里是 09:12 交来的提案。
    expect(oldestWaitMs).toBe(NOW - submitted);
    // 读到列表的时刻换了，草案等的时长跟着变，黑板任务不变。
    const later = plan({ policy: policyReady(data.desired, NOW + 60_000) });
    expect(later.entries[0].waitMs).toBe(NOW + 60_000 - submitted);
    expect(later.entries[1].waitMs).toBe(waits.get(`board:${TASK.T093}`));
    // 读到的时刻比提交还早（本机钟慢）：不出负数。
    expect(
      plan({
        policy: policyReady(data.desired, submitted - 5_000),
      }).entries.find((entry) => entry.source === 'policy')!.waitMs,
    ).toBe(0);
  });

  it('类型分布按在队列里第一次出现的先后数', () => {
    expect(plan({ inbox: [] }).kinds).toEqual([
      { kind: KIND_PROPOSAL, count: 1 },
      { kind: '待修订', count: 1 },
      { kind: '待验收', count: 2 },
    ]);
    // 工作台待办的类型和黑板的同名时并成一类。
    const merged = plan({ inbox: fixture.inbox }).kinds;
    expect(merged).toContainEqual({ kind: '待验收', count: 3 });
    expect(merged.reduce((sum, entry) => sum + entry.count, 0)).toBe(5);
  });

  it('不带时间的接在带时间的后面：先黑板、再策略、最后工作台待办', () => {
    const silent = { ...item(TASK.T106), progress: null };
    const board: OverviewState = {
      kind: 'ready',
      overview: {
        ...data.overview,
        items: data.overview.items.map((entry) =>
          entry.id === TASK.T106 ? silent : entry,
        ),
      },
    };
    const undated = draft({
      id: '01M33ZA2G0HQ8K8HMTV4NH3KKQ',
      subject: 'codex',
      submitted_at: '时间不详',
    });
    const { entries, oldestWaitMs } = plan({
      board,
      policy: policyReady([proposal, undated]),
      inbox: fixture.inbox,
    });
    expect(entries.map(entryKey)).toEqual([
      `policy:${DESIRED_ID}`,
      `board:${TASK.T093}`,
      `board:${TASK.T108}`,
      `board:${TASK.T106}`,
      `policy:${undated.id}`,
      'inbox:action-1',
    ]);
    expect(entries.map((entry) => entry.waitMs === null)).toEqual([
      false,
      false,
      false,
      true,
      true,
      true,
    ]);
    expect(oldestWaitMs).toBe(NOW - Date.parse(proposal.submitted_at));
  });

  it('同一刻开始等的保持来源先后：黑板在前，策略在后', () => {
    const since = item(TASK.T093).progress!.lastProgressMs;
    const tied = draft({ submitted_at: new Date(since).toISOString() });
    expect(
      plan({ policy: policyReady([tied]), inbox: [] })
        .entries.slice(0, 2)
        .map(entryKey),
    ).toEqual([`board:${TASK.T093}`, `policy:${DESIRED_ID}`]);
  });

  it('只有工作台待办时没有「最久」', () => {
    const only = plan({
      board: { kind: 'disabled' },
      policy: { kind: 'absent' },
      inbox: fixture.inbox,
      projects: fixture.projects,
    });
    expect(only.oldestWaitMs).toBeNull();
    expect(only.entries.map(entryKey)).toEqual(['inbox:action-1']);
    expect(only.entries[0]).toMatchObject({
      n: 1,
      kind: '待验收',
      project: '星舟',
      since: null,
    });
    expect(only.total).toBe(1);
  });

  it('汇总被截断时照实标出', () => {
    const truncated = plan({
      board: {
        kind: 'ready',
        overview: { ...data.overview, itemsTruncated: true },
      },
    });
    expect(truncated.truncated).toBe(true);
    expect(plan().truncated).toBe(false);
    expect(plan({ board: { kind: 'disabled' } }).truncated).toBe(false);
  });

  it('待办标题以「类型：」开头时去掉重复的类型前缀；类型原文为空写「未知」', () => {
    const [action] = fixture.inbox;
    expect(actionTitle({ ...action, title: '待验收：验收只读工作台' })).toBe(
      '验收只读工作台',
    );
    expect(actionTitle(action)).toBe('验收只读工作台');
    const blank = plan({
      board: { kind: 'disabled' },
      policy: { kind: 'absent' },
      inbox: [{ ...action, kind: '' }],
    });
    expect(blank.entries[0].kind).toBe('未知');
    expect(blank.kinds).toEqual([{ kind: '未知', count: 1 }]);
  });
});

describe('策略里哪些算等你', () => {
  it('待复核和已校验的都算；AI 交的叫「AI 提案」，你自己交的叫「待发布草案」', () => {
    const own = draft({
      id: '01M33ZA2G0HQ8K8HMTV4NH3KKR',
      subject: 'codex',
      status: 'validated',
      submitter_kind: 'owner',
      agent_type: null,
      agent_instance: null,
    });
    expect(waitingDesired([proposal, own])).toEqual([proposal, own]);
    expect(
      plan({
        board: { kind: 'disabled' },
        policy: policyReady([proposal, own]),
      }).entries.flatMap((entry) =>
        entry.source === 'policy' ? [[entry.desired.id, entry.kind]] : [],
      ),
    ).toEqual([
      [proposal.id, KIND_PROPOSAL],
      [own.id, KIND_DRAFT],
    ]);
  });

  it('校验没通过的、已发布的、已取代的、已撤回的都不算', () => {
    for (const status of [
      'rejected',
      'activated',
      'superseded',
      'withdrawn',
    ] as const)
      expect(waitingDesired([draft({ status })])).toEqual([]);
  });

  it('被同一提交者更新的草案挡住的较早那份不算：要处理的是更新的那份', () => {
    const newer = draft({ id: '01M33ZA2G0HQ8K8HMTV4NH3KKS' });
    const older = draft({ newer_open: [newer.id], blocked_by: [newer.id] });
    expect(waitingDesired([newer, older])).toEqual([newer]);
    // 别的提交者交了更新的一份、没挡住发布：两份都等你。
    const unblocked = draft({ newer_open: [newer.id], blocked_by: [] });
    expect(waitingDesired([newer, unblocked])).toEqual([newer, unblocked]);
    expect(
      pendingCount(
        { inbox: [] },
        { kind: 'disabled' },
        policyReady([newer, older]),
      ),
    ).toBe(1);
  });
});

describe('等你的总数：巨号与顶栏「驾驶舱 N」角标同一个函数（T-162、T-195）', () => {
  it.each<[string, OverviewState, PolicyPendingState, number | null]>([
    ['三路都读到', ready, drafts, inbox.length + 3 + 1],
    ['黑板未启用（404）', { kind: 'disabled' }, drafts, inbox.length + 1],
    ['没装策略平面（404）', ready, { kind: 'absent' }, inbox.length + 3],
    [
      '黑板和策略都没有',
      { kind: 'disabled' },
      { kind: 'absent' },
      inbox.length,
    ],
    ['黑板读取中', { kind: 'loading' }, drafts, null],
    ['黑板读取失败', BOARD_FAILED, drafts, null],
    ['策略读取中', ready, { kind: 'loading' }, null],
    ['策略读取失败', ready, POLICY_FAILED, null],
    ['两路都在读', { kind: 'loading' }, { kind: 'loading' }, null],
  ])('%s', (_, board, policy, expected) => {
    expect(pendingCount(data.workbench, board, policy)).toBe(expected);
    // 巨号用的 plan.total 就是它：两处不会各算各的。
    expect(plan({ board, policy }).total).toBe(expected);
  });

  it('总数不可知时，已经读到的照常列出，不拿它们冒充总数', () => {
    const boardOnly = plan({ policy: { kind: 'loading' } });
    expect(boardOnly.total).toBeNull();
    expect(boardOnly.entries).toHaveLength(3 + inbox.length);
    const policyOnly = plan({ board: BOARD_FAILED });
    expect(policyOnly.total).toBeNull();
    expect(policyOnly.entries).toHaveLength(1 + inbox.length);
    expect(policyOnly.entries[0].source).toBe('policy');
  });

  it('只数未结束、等你的黑板任务：已认领、在跑、已批准与已结束的都不算', () => {
    // 假数据汇总 8 条：3 条等你（T-093 待修订、T-106 与 T-108 待验收）；
    // T-046 已认领、T-109 与 T-130 在跑、T-092 已批准、简报站 T-021 已结束，都不算。
    expect(data.overview.items).toHaveLength(8);
    expect(pendingCount({ inbox: [] }, ready, { kind: 'absent' })).toBe(3);
    expect(
      plan({ inbox: [], policy: { kind: 'absent' } }).entries.map(
        (entry) => entry.source,
      ),
    ).toEqual(['board', 'board', 'board']);
  });

  it('还没有工作台快照：不论另外两路读到哪一步，总数都不可知', () => {
    const boards: OverviewState[] = [
      ready,
      { kind: 'disabled' },
      { kind: 'loading' },
      BOARD_FAILED,
    ];
    const policies: PolicyPendingState[] = [
      drafts,
      { kind: 'absent' },
      { kind: 'loading' },
      POLICY_FAILED,
    ];
    for (const board of boards)
      for (const policy of policies)
        expect(pendingCount(null, board, policy)).toBeNull();
  });
});
