import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { healthBody } from '../../dev/fixtures/health';
import { HealthPanel } from '../HealthPanel';
import { BACKUP_JOBS, parseHealth, type HealthReport } from '../healthApi';
import { parseBackupJobs } from '../health/model';
import { duration } from '../health/format';

const copy = () => structuredClone(healthBody());
const section = (report: HealthReport) =>
  report.sections.find((entry) => entry.code === BACKUP_JOBS);

describe('备份作业明细', () => {
  it('和本节结论对得上的明细原样保留，64位序号不转浮点数', () => {
    const body = copy();
    body.backup.last_success.sequence = '18446744073709551615';
    const report = parseHealth(body);
    expect(section(report)?.state).toBe('normal');
    expect(report.backup?.verdict).toBe('healthy');
    expect(report.backup?.lastSuccess?.sequence).toBe('18446744073709551615');
    expect(report.backup?.lastFailure).toMatchObject({
      result: 'failed',
      reason: 'source_changed_before_seal',
    });
  });

  it('形状不对、原因认不出、结论和明细矛盾，整份都不认', () => {
    const base = copy().backup;
    for (const backup of [
      { ...base, arbitrary: true },
      { ...base, state: 'degraded' },
      { ...base, verdict: 'stuck', state: 'degraded' },
      { ...base, overdue: true },
      { ...base, latest_failed: true, verdict: 'healthy' },
      { ...base, last_success: null },
      { ...base, outstanding: { ...base.outstanding, stuck: 'no' } },
      {
        ...base,
        last_failure: { ...base.last_failure, reason: 'probably_fine' },
      },
      {
        ...base,
        last_failure: { ...base.last_failure, result: 'unrecognized' },
      },
      {
        ...base,
        last_failure: { ...base.last_failure, result: 'unverified' },
      },
      { ...base, stuck_after_seconds: 86400 },
      { ...base, overdue_after_seconds: '18446744073709551616' },
      { ...base, as_of: 'yesterday' },
    ])
      expect(parseBackupJobs(backup)).toBeNull();
  });

  it('卡住、逾期、最近失败、判断不了、从没成功都按后端同一顺序核对', () => {
    const base = copy().backup;
    const stuck = { ...base.outstanding, stuck: true };
    expect(
      parseBackupJobs({
        ...base,
        state: 'degraded',
        verdict: 'stuck',
        outstanding: stuck,
      })?.verdict,
    ).toBe('stuck');
    expect(
      parseBackupJobs({
        ...base,
        state: 'degraded',
        verdict: 'overdue',
        overdue: true,
      })?.verdict,
    ).toBe('overdue');
    // 又卡住又逾期：卡住在前，自称逾期的不认。
    expect(
      parseBackupJobs({
        ...base,
        state: 'degraded',
        verdict: 'overdue',
        overdue: true,
        outstanding: stuck,
      }),
    ).toBeNull();
    expect(
      parseBackupJobs({
        ...base,
        state: 'degraded',
        verdict: 'latest_failed',
        latest_failed: true,
      })?.latestFailed,
    ).toBe(true);
    expect(
      parseBackupJobs({
        ...base,
        state: 'degraded',
        verdict: 'latest_failed',
        latest_failed: true,
        last_failure: null,
      }),
    ).toBeNull();
    expect(
      parseBackupJobs({
        ...base,
        state: 'unknown',
        verdict: 'clock_behind',
        overdue: null,
      })?.overdue,
    ).toBeNull();
    expect(
      parseBackupJobs({
        ...base,
        state: 'unknown',
        verdict: 'no_success_yet',
        last_success: null,
      })?.lastSuccess,
    ).toBeNull();
  });

  it('明细对不上、缺了却报正常时，本节降为未知，明细丢掉', () => {
    const contradicting = copy();
    contradicting.backup.state = 'degraded';
    let report = parseHealth(contradicting);
    expect(section(report)?.state).toBe('unknown');
    expect(report.backup).toBeNull();
    expect(report.evidenceComplete).toBe(false);

    const mismatched = copy();
    mismatched.sections = mismatched.sections.map((entry) =>
      entry.code === BACKUP_JOBS ? { ...entry, state: 'degraded' } : entry,
    );
    report = parseHealth(mismatched);
    expect(section(report)?.state).toBe('unknown');
    expect(report.backup).toBeNull();

    const missing: Record<string, unknown> = copy();
    delete missing.backup;
    report = parseHealth(missing);
    expect(section(report)?.state).toBe('unknown');
    expect(report.backup).toBeNull();
  });

  it('没启用时本节如实未知、没有明细，不算形状错', () => {
    const disabled: Record<string, unknown> = copy();
    disabled.backup = null;
    disabled.sections = copy().sections.map((entry) =>
      entry.code === BACKUP_JOBS
        ? {
            ...entry,
            state: 'unknown',
            detail: '备份生产者没有启用：私有配置里没有备份那两段，看不到作业',
          }
        : entry,
    );
    const report = parseHealth(disabled);
    expect(section(report)?.state).toBe('unknown');
    expect(report.backup).toBeNull();
    expect(
      report.sections.find((entry) => entry.code === 'sync-bridge')?.state,
    ).toBe('degraded');
    render(<HealthPanel report={report} />);
    expect(screen.queryByText('查看备份作业')).toBeNull();
  });

  it('面板里看得到最近成功、挂着的作业、逾期起算和最近失败的中文原因', () => {
    render(<HealthPanel report={parseHealth(copy())} />);
    const row = screen
      .getByRole('heading', { name: '备份作业' })
      .closest('li') as HTMLElement;
    expect(within(row).getByText('查看备份作业')).toBeTruthy();
    expect(within(row).getByText(/^第 42 号 · /)).toBeTruthy();
    expect(
      within(row).getByText(/^第 43 号 · .+已封存，等 worker 上传 · 没卡住$/),
    ).toBeTruthy();
    expect(within(row).getByText(/^没有逾期（从 .+ 起算）$/)).toBeTruthy();
    expect(
      within(row).getByText(/^第 41 号 · .+ · 封存前来源变了（之后已有成功）$/),
    ).toBeTruthy();
    expect(within(row).getByText('挂过保留期 1 天')).toBeTruthy();
    expect(within(row).getByText('两倍最小间隔 2 小时 没有成功')).toBeTruthy();
  });

  it('时长写法和后端正文一致', () => {
    expect(duration('0')).toBe('不到 1 分钟');
    expect(duration('59')).toBe('不到 1 分钟');
    expect(duration('300')).toBe('5 分钟');
    expect(duration('7200')).toBe('2 小时');
    expect(duration('7500')).toBe('2 小时 5 分钟');
    expect(duration('86400')).toBe('1 天');
    expect(duration('93600')).toBe('1 天 2 小时');
    expect(duration('18446744073709551615')).toMatch(/^\d+ 天( \d+ 小时)?$/);
  });
});
