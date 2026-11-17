import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { healthBody } from '../../dev/fixtures/health';
import { HealthPanel } from '../HealthPanel';
import { parseHealth } from '../healthApi';
import { parseBridge, parseManualQuota } from '../health/model';
import { bytes } from '../health/format';

const copy = () => structuredClone(healthBody());
describe('同步健康结构与人工记录', () => {
  it('保留同步证据与独立人工账号用量，64位计数不转成浮点数', () => {
    const body = copy();
    body.bridge.unknown_jobs = '18446744073709551615';
    const report = parseHealth(body);
    expect(report.bridge?.unknownJobs).toBe('18446744073709551615');
    expect(report.bridge?.mirrorBytes).toBe('1288490188');
    expect(report.manualSyncQuota?.usedBytesManual).toBe('5368709120');
    expect(bytes('18446744073709551615')).toBe('17179869183.9 GiB');
  });
  it('未知结构、超出u64或未知状态不会被当成同步证据', () => {
    for (const bridge of [
      { ...copy().bridge, arbitrary: true },
      { ...copy().bridge, unknown_jobs: '18446744073709551616' },
      { ...copy().bridge, unknown_jobs: 2 },
      { ...copy().bridge, last_exit: 'probably_ok' },
      {
        ...copy().bridge,
        last_batch: { sequence: '2', closed_at: 'yesterday' },
      },
    ])
      expect(parseBridge(bridge)).toBeNull();
  });
  it('正常声明与熔断事实矛盾时降为未知，保留真实明细', () => {
    const body = copy();
    body.overall = 'normal';
    body.sections = body.sections.map((section) => ({
      ...section,
      state: 'normal',
    }));
    body.bridge.fused = true;
    const report = parseHealth(body);
    expect(report.overall).toBe('unknown');
    expect(report.evidenceComplete).toBe(false);
    expect(
      report.sections.find((section) => section.code === 'sync-bridge')?.state,
    ).toBe('unknown');
    expect(report.bridge?.fused).toBe(true);
  });
  it('70%边界、过期和未来记录都必须匹配结论，不能伪装成正常', () => {
    const quota = {
      ...copy().manual_sync_quota,
      account_limit_bytes: '1000',
      used_bytes_manual: '700',
      state: 'degraded',
      reason: 'warning',
    };
    expect(parseManualQuota(quota)?.reason).toBe('warning');
    expect(
      parseManualQuota({ ...quota, state: 'normal', reason: 'within' }),
    ).toBeNull();
    expect(
      parseManualQuota({
        ...quota,
        recorded_at: '2026-11-10T12:00:00Z',
        state: 'unknown',
        reason: 'stale',
      })?.reason,
    ).toBe('stale');
    expect(
      parseManualQuota({
        ...quota,
        recorded_at: '2026-11-14T12:00:00Z',
        state: 'unknown',
        reason: 'clock',
      })?.reason,
    ).toBe('clock');
    expect(
      parseManualQuota({ ...quota, recorded_at: '2026-11-14T12:00:00Z' }),
    ).toBeNull();
  });
  it('原健康结构里的缺失、多余和重复字段也不显示正常', () => {
    const section = {
      code: 'sync-bridge',
      title: '双向 Bridge',
      state: 'normal',
      detail: '正常',
    };
    for (const body of [
      {
        overall: 'normal',
        evidence_complete: true,
        sections: [{ ...section, unexpected: 'extra' }],
      },
      {
        overall: 'normal',
        evidence_complete: true,
        sections: [section, section],
      },
      { overall: 'normal', evidence_complete: false, sections: [section] },
      {
        overall: 'normal',
        evidence_complete: true,
        sections: [section],
        unexpected: true,
      },
      {
        overall: 'normal',
        evidence_complete: true,
        sections: [{ code: 'sync-bridge', state: 'normal' }],
      },
    ]) {
      expect(parseHealth(body).overall).toBe('unknown');
      expect(parseHealth(body).evidenceComplete).toBe(false);
    }
  });
  it('同步桥或人工额度报正常却没有明细时降为未知，与备份作业同一口径', () => {
    const normal = (code: string) => ({
      code,
      title: code,
      state: 'normal',
      detail: '正常',
    });
    const base = { overall: 'normal', evidence_complete: true };
    for (const extra of [
      { bridge: null, manual_sync_quota: null, backup: null },
      {},
    ]) {
      const report = parseHealth({
        ...base,
        ...extra,
        sections: [normal('sync-bridge'), normal('manual-sync-quota')],
      });
      expect(report.sections.map((section) => section.state)).toEqual([
        'unknown',
        'unknown',
      ]);
      expect(report.overall).toBe('unknown');
      expect(report.evidenceComplete).toBe(false);
    }
    // 对照：别的节没有对应明细，报正常照旧是正常。
    expect(
      parseHealth({ ...base, sections: [normal('disk-quota')] }).overall,
    ).toBe('normal');
  });
  it('已有降级证据优先展示，不被未知项洗掉', () => {
    const body = copy();
    body.bridge = { ...body.bridge, unknown_jobs: 'not_a_number' };
    body.sections.push({
      code: 'disk-failure',
      title: '磁盘',
      state: 'degraded',
      detail: '空间不足',
    });
    const report = parseHealth(body);
    expect(report.overall).toBe('degraded');
    expect(report.evidenceComplete).toBe(false);
  });
  it('浮层展示可展开的全部同步证据、人工时间和非账号用量声明', () => {
    render(<HealthPanel report={parseHealth(copy())} />);
    fireEvent.click(screen.getByText('查看同步证据'));
    expect(screen.getByText('终态未知动作')).toBeInTheDocument();
    expect(screen.getByText('冲突副本')).toBeInTheDocument();
    expect(screen.getByText('1.1 GiB（非账号用量）')).toBeInTheDocument();
    expect(
      screen.queryByText('非零退出', { exact: true }),
    ).not.toBeInTheDocument();
    expect(screen.getByText('超过时限')).toBeInTheDocument();
    expect(screen.getByText(/5.0 GiB \/ 10.0 GiB/)).toBeInTheDocument();
    expect(screen.getByText(/超过24小时需重新核对/)).toBeInTheDocument();
    expect(
      screen.getByText('实时账号用量未知；本机镜像大小单独列出。'),
    ).toBeInTheDocument();
  });
});
