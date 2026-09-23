import { describe, expect, it } from 'vitest';
import type { OverviewState } from '../../cockpit/model';
import { drawingSubtitle, titleCells } from '../../cockpit/readings';
import { freshnessReason } from '../../freshness';
import type { HealthStatus } from '../../shell/useHealthStatus';
import type { TitleBlockCell } from '../../ui';
import { fixture } from '../fixtures';
import { fixtureData } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const healthy: HealthStatus = { kind: 'ready', report: data.health };

function cells(
  overrides: Partial<Parameters<typeof titleCells>[0]> = {},
): Record<string, TitleBlockCell> {
  return Object.fromEntries(
    titleCells({
      workbench: data.workbench,
      health: healthy,
      overview: ready,
      ...overrides,
    }).map((cell) => [String(cell.label), cell]),
  );
}

describe('标题栏读数', () => {
  it('十二格按规范第 6 节的顺序排', () => {
    expect(
      titleCells({
        workbench: data.workbench,
        health: healthy,
        overview: ready,
      }).map((cell) => cell.label),
    ).toEqual([
      '副本新鲜度',
      '每轮同步',
      '同步时限',
      '黑板在跑',
      'agent 程序',
      '人侧发行',
      '会话',
      '执行',
      '可读项目',
      '副本文件',
      '副本大小',
      '未关闭 BUG',
    ]);
  });

  it('每格取接口原值：工作台计数、黑板汇总条数、健康报告原文里的数字', () => {
    const all = cells();
    expect(all['可读项目'].value).toBe('8');
    expect(all['副本文件'].value).toBe(
      String(data.workbench.counts.documents.value),
    );
    // 未结束 7 条；尝试仍在进行的 3 条（T-046 认领、T-109 与 T-130 在跑）。
    expect(all['黑板在跑'].value).toBe('7');
    expect(all['执行'].value).toBe('3');
    // 健康「副本同步」一节降级：每轮标红，时限不标红；完整原文进原因。
    expect(all['每轮同步']).toMatchObject({
      value: '85 S',
      tone: 'signal',
      reason:
        '副本同步：每轮 85 秒，时限 120 秒；余量不足三成，全量重推不可持续',
    });
    expect(all['同步时限'].value).toBe('120 S');
    expect(all['同步时限'].tone).toBeUndefined();
    expect(all['会话']).toMatchObject({
      value: '今日已复核',
      reason: '今日已复核，当前会话 12 小时内有效',
    });
  });

  it('接口不给的读数显示「—」并写明原因，绝不编造', () => {
    const all = cells();
    for (const [label, reason] of [
      ['agent 程序', '接口未提供 agent 程序版本。'],
      ['人侧发行', '接口未提供人侧发行号。'],
      ['副本大小', '接口未提供副本大小。'],
    ] as const)
      expect(all[label]).toEqual({ label, value: null, reason });
    // 未关闭 BUG：有项目没有 BUG 档，汇总显式不可得，逐个列出。
    const bugs = all['未关闭 BUG'];
    expect(bugs.value).toBeNull();
    const unavailable = data.workbench.counts.open_bugs.unavailable;
    expect(bugs.reason).toBe(
      `${unavailable.length} 个项目的未关闭 BUG 数不可得：${unavailable.join('、')}。`,
    );
  });

  it('副本新鲜度：陈旧时标红，读数是两条水位之差，原因取 freshness 原文', () => {
    const freshness = data.workbench.freshness!;
    expect(cells()['副本新鲜度']).toEqual({
      label: '副本新鲜度',
      value: '落后 3 条',
      tone: 'signal',
      reason: `${freshnessReason(freshness)}索引水位 ${freshness.indexed_through}，提交水位 ${freshness.committed_through}。`,
    });
    const caughtUp = {
      ...data.workbench,
      freshness: {
        ...freshness,
        stale: false,
        reason: null,
        indexed_through: freshness.committed_through,
      },
    };
    expect(cells({ workbench: caughtUp })['副本新鲜度']).toEqual({
      label: '副本新鲜度',
      value: '已追平',
    });
    const unknownIndex = {
      ...data.workbench,
      freshness: { ...freshness, indexed_through: null },
    };
    expect(cells({ workbench: unknownIndex })['副本新鲜度'].value).toBe(
      '未追平',
    );
    expect(
      cells({ workbench: { ...data.workbench, freshness: undefined } })[
        '副本新鲜度'
      ],
    ).toEqual({
      label: '副本新鲜度',
      value: null,
      reason: '工作台快照没有给出新鲜度证据。',
    });
  });

  it('本机健康读取中、失败、没有对应一节，各自写明原因', () => {
    const loading = cells({ health: { kind: 'loading' } });
    expect(loading['每轮同步']).toMatchObject({
      value: null,
      reason: '本机健康读取中。',
    });
    const failed = cells({
      health: { kind: 'error', message: '请求失败（404）' },
    });
    expect(failed['会话']).toMatchObject({
      value: null,
      reason: '本机健康读取失败：请求失败（404）',
    });
    const bare = cells({
      health: {
        kind: 'ready',
        report: {
          ...data.health,
          sections: data.health.sections.filter(
            (section) => !['sync-round', 'session'].includes(section.code),
          ),
        },
      },
    });
    for (const label of ['每轮同步', '同步时限', '会话'])
      expect(bare[label]).toMatchObject({
        value: null,
        reason: '健康报告未提供',
      });
  });

  it('健康原文不是「每轮 N 秒」的说法时不猜数字，原文进原因', () => {
    const reworded = cells({
      health: {
        kind: 'ready',
        report: {
          ...data.health,
          sections: data.health.sections.map((section) =>
            section.code === 'sync-round'
              ? { ...section, detail: '同步耗时接近上限' }
              : section,
          ),
        },
      },
    });
    expect(reworded['每轮同步']).toEqual({
      label: '每轮同步',
      value: null,
      reason: '健康报告没有给出每轮同步的秒数。副本同步：同步耗时接近上限',
    });
  });

  it('黑板相关两格：未启用、读取中、失败各写原因；截断时读数带「+」并说明', () => {
    for (const [state, reason] of [
      [{ kind: 'disabled' }, '黑板未启用'],
      [{ kind: 'loading' }, '黑板汇总读取中。'],
      [
        { kind: 'error', message: '黑板许可已满。' },
        '黑板汇总读取失败：黑板许可已满。',
      ],
    ] as const) {
      const all = cells({ overview: state });
      expect(all['黑板在跑']).toEqual({
        label: '黑板在跑',
        value: null,
        reason,
      });
      expect(all['执行']).toEqual({ label: '执行', value: null, reason });
    }
    const truncated = cells({
      overview: {
        kind: 'ready',
        overview: { ...data.overview, itemsTruncated: true },
      },
    });
    expect(truncated['黑板在跑']).toEqual({
      label: '黑板在跑',
      value: '7+',
      reason: '黑板汇总已截断（最多 500 条），实际可能更多。',
    });
  });

  it('计数：0 是真读数；不可得时列出是哪些项目', () => {
    const empty = {
      ...fixture,
      counts: {
        ...fixture.counts,
        documents: { value: 0, unavailable: [] },
      },
    };
    const all = cells({ workbench: empty });
    expect(all['副本文件'].value).toBe('0');
    expect(all['未关闭 BUG'].reason).toBe(
      '1 个项目的未关闭 BUG 数不可得：灵感盒。',
    );
    const bare = cells({
      workbench: {
        ...fixture,
        counts: {
          ...fixture.counts,
          documents: { value: null, unavailable: [] },
        },
      },
    });
    expect(bare['副本文件']).toEqual({
      label: '副本文件',
      value: null,
      reason: '服务端没有给出文档数。',
    });
    const many = cells({
      workbench: {
        ...fixture,
        counts: {
          ...fixture.counts,
          documents: { value: 1382, unavailable: [] },
        },
      },
    });
    expect(many['副本文件'].value).toBe('1,382');
  });
});

describe('标题栏副题', () => {
  it('日期带星期，换行写副本代次；刷新中注明', () => {
    const tuesday = new Date(2026, 8, 22, 20, 31).getTime();
    expect(drawingSubtitle(tuesday, 243, false)).toBe(
      '2026-09-22 周二\n副本代次 243',
    );
    expect(drawingSubtitle(tuesday, 243, true)).toBe(
      '2026-09-22 周二\n副本代次 243（刷新中）',
    );
    expect(drawingSubtitle(new Date(2026, 8, 27).getTime(), 7, false)).toBe(
      '2026-09-27 周日\n副本代次 7',
    );
  });
});
