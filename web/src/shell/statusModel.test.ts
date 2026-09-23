import { describe, expect, it } from 'vitest';
import type { Freshness } from '../freshness';
import type { HealthReport } from '../healthApi';
import { summarizeStatus } from './statusModel';
import type { StatusInputs } from './statusModel';
import type { HealthStatus } from './useHealthStatus';

const fresh: Freshness = {
  stale: false,
  indexed_through: '12',
  committed_through: '12',
  unresolved: false,
  watcher_healthy: true,
  reason: null,
};

function report(overrides: Partial<HealthReport> = {}): HealthStatus {
  return {
    kind: 'ready',
    report: {
      overall: 'normal',
      evidenceComplete: true,
      sections: [
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'normal',
          detail: '占用在预警水位以内',
        },
      ],
      ...overrides,
    },
  };
}

function inputs(overrides: Partial<StatusInputs> = {}): StatusInputs {
  return {
    loaded: true,
    freshness: fresh,
    refreshing: false,
    failed: false,
    health: report(),
    diagnostics: 0,
    ...overrides,
  };
}

describe('状态胶囊的一句话', () => {
  it('新鲜度、本机健康与数据提示全部正常才说正常', () => {
    expect(summarizeStatus(inputs())).toEqual({
      level: 'normal',
      label: '正常',
    });
  });

  it('读取或核验在途是空心方块，不冒充正常', () => {
    expect(
      summarizeStatus(inputs({ loaded: false, refreshing: true })),
    ).toEqual({ level: 'pending', label: '读取中' });
    expect(summarizeStatus(inputs({ refreshing: true }))).toEqual({
      level: 'pending',
      label: '核验中',
    });
    expect(summarizeStatus(inputs({ health: { kind: 'loading' } }))).toEqual({
      level: 'pending',
      label: '读取健康',
    });
  });

  it('工作台读取失败按有无快照分别说明', () => {
    expect(summarizeStatus(inputs({ loaded: false, failed: true }))).toEqual({
      level: 'problem',
      label: '无法读取工作台',
    });
    expect(summarizeStatus(inputs({ failed: true }))).toEqual({
      level: 'problem',
      label: '核验失败',
    });
  });

  it.each([undefined, null])('服务没给新鲜度证据（%s）不算正常', (value) => {
    expect(summarizeStatus(inputs({ freshness: value }))).toEqual({
      level: 'problem',
      label: '无新鲜度证据',
    });
  });

  it.each([
    ['initializing', '首次快照准备中'],
    ['refreshing', '服务核验中'],
    ['periodic_check', '周期核验中'],
    ['changed', '内容已变化'],
    ['watcher_rescan', '目录待重扫'],
    ['watcher_failed', '目录监视失效'],
    ['capture_failed', '内容读取未完成'],
    ['build_failed', '快照构建失败'],
    ['binding_changed', '目录绑定变化'],
    ['unresolved', '写入结果未决'],
    ['writer_state_invalid', '写者状态不明'],
    ['stopping', '服务停止中'],
    ['unavailable', '状态暂不可确认'],
    ['future_check', '快照陈旧'],
    ['constructor', '快照陈旧'],
  ])('快照陈旧的原因 %s 报成「%s」', (reason, label) => {
    expect(
      summarizeStatus(inputs({ freshness: { ...fresh, stale: true, reason } })),
    ).toEqual({ level: 'problem', label });
  });

  it('没有原因码时按未决写入、目录监视的顺序推断，推断不出就说未追平', () => {
    const stale = { ...fresh, stale: true };
    expect(
      summarizeStatus(inputs({ freshness: { ...stale, unresolved: true } }))
        .label,
    ).toBe('写入结果未决');
    expect(
      summarizeStatus(
        inputs({ freshness: { ...stale, watcher_healthy: false } }),
      ).label,
    ).toBe('目录监视失效');
    expect(summarizeStatus(inputs({ freshness: stale })).label).toBe(
      '快照未追平',
    );
  });

  it('本机健康：未知、读取失败、证据不全都不是正常；任一检查项降级就是降级', () => {
    expect(
      summarizeStatus(inputs({ health: { kind: 'error', message: '断开' } })),
    ).toEqual({ level: 'problem', label: '健康未知' });
    expect(
      summarizeStatus(
        inputs({
          health: report({ overall: 'unknown', evidenceComplete: false }),
        }),
      ).label,
    ).toBe('健康未知');
    expect(
      summarizeStatus(inputs({ health: report({ evidenceComplete: false }) }))
        .label,
    ).toBe('健康证据不全');
    expect(
      summarizeStatus(inputs({ health: report({ overall: 'degraded' }) }))
        .label,
    ).toBe('健康降级');
    expect(
      summarizeStatus(
        inputs({
          health: report({
            sections: [
              {
                code: 'sync-quota',
                title: 'Sync 配额',
                state: 'degraded',
                detail: '占用已越过上限',
              },
            ],
          }),
        }),
      ).label,
    ).toBe('健康降级');
  });

  it('有数据提示就报条数', () => {
    expect(summarizeStatus(inputs({ diagnostics: 3 }))).toEqual({
      level: 'problem',
      label: '数据提示 3',
    });
  });

  it('多项同时出问题只报最严重的一项，问题压过在途', () => {
    const everything = inputs({
      failed: true,
      freshness: { ...fresh, stale: true, reason: 'changed' },
      health: report({ overall: 'degraded' }),
      diagnostics: 2,
    });
    expect(summarizeStatus(everything).label).toBe('核验失败');
    expect(summarizeStatus({ ...everything, failed: false }).label).toBe(
      '健康降级',
    );
    expect(
      summarizeStatus({ ...everything, failed: false, health: report() }).label,
    ).toBe('内容已变化');
    expect(
      summarizeStatus(
        inputs({
          freshness: { ...fresh, stale: true, reason: 'changed' },
          health: { kind: 'error', message: '断开' },
        }),
      ).label,
    ).toBe('内容已变化');
    expect(
      summarizeStatus(
        inputs({
          freshness: null,
          health: report({ evidenceComplete: false }),
          diagnostics: 1,
        }),
      ).label,
    ).toBe('健康证据不全');
    expect(
      summarizeStatus(inputs({ freshness: null, diagnostics: 1 })).label,
    ).toBe('无新鲜度证据');
    expect(
      summarizeStatus(inputs({ refreshing: true, diagnostics: 1 })),
    ).toEqual({ level: 'problem', label: '数据提示 1' });
    expect(
      summarizeStatus(
        inputs({
          loaded: false,
          refreshing: true,
          health: { kind: 'loading' },
        }),
      ).label,
    ).toBe('读取中');
  });
});
