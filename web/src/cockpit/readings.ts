/**
 * 标题栏（T-130 设计规范第 6 节第 6 条）：图名、日期与副本代次，十二格读数按规范顺序。
 *
 * 每格只取接口原值；拿不到就是 null（显示「—」），原因写进 reason（格子的 title 与读屏文字）。
 * 健康报告里的同步与会话读数目前只有开发假数据给出（后端尚未发这几节）：按 section code 找，
 * 找到才从原文里摘出数字或首句，完整原文进 reason；找不到就写「健康报告未提供」。
 */
import { freshnessReason, type Freshness } from '../freshness';
import type { HealthSection } from '../healthApi';
import type { HealthStatus } from '../shell/useHealthStatus';
import type { TitleBlockCell } from '../ui';
import type { CountRollup, Workbench } from '../types';
import { isFinished, type OverviewState } from './model';

/** 健康报告里给标题栏的几节（code 与开发假数据 `web/dev/fixtures/health.ts` 一致）。 */
export const HEALTH_SYNC = 'sync-round';
export const HEALTH_SESSION = 'session';

const WEEKDAYS = ['日', '一', '二', '三', '四', '五', '六'];
const pad = (value: number) => String(value).padStart(2, '0');
const counts = new Intl.NumberFormat('en-US');

/** 标题栏副题：`2026-09-22 周二` 换行 `副本代次 243`，刷新中在末尾注明。日期取本机今天。 */
export function drawingSubtitle(
  now: number,
  generation: number,
  refreshing: boolean,
): string {
  const date = new Date(now);
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )} 周${WEEKDAYS[date.getDay()]}`;
  return `${day}\n副本代次 ${generation}${refreshing ? '（刷新中）' : ''}`;
}

const missing = (label: string, reason: string): TitleBlockCell => ({
  label,
  value: null,
  reason,
});

/** 两个十进制水位相减（u64 以字符串给出，用 BigInt 免得超过 2^53 失真）；任一不可比时为 null。 */
function lag(freshness: Freshness): bigint | null {
  const { indexed_through: indexed, committed_through: committed } = freshness;
  if (indexed === null || committed === null) return null;
  const behind = BigInt(committed) - BigInt(indexed);
  return behind > 0n ? behind : null;
}

function freshnessCell(
  freshness: Freshness | null | undefined,
): TitleBlockCell {
  const label = '副本新鲜度';
  if (!freshness) return missing(label, '工作台快照没有给出新鲜度证据。');
  if (!freshness.stale) return { label, value: '已追平' };
  const behind = lag(freshness);
  return {
    label,
    value: behind === null ? '未追平' : `落后 ${counts.format(behind)} 条`,
    tone: 'signal',
    // 原因取 freshness.ts 的原文，再附两条水位，读数「落后 N 条」由此而来。
    reason: `${freshnessReason(freshness)}索引水位 ${
      freshness.indexed_through ?? '未知'
    }，提交水位 ${freshness.committed_through ?? '未知'}。`,
  };
}

/** 健康报告的一节：读取中、失败、没有这一节，各给各的原因。 */
function healthSection(
  health: HealthStatus,
  code: string,
): { section: HealthSection } | { reason: string } {
  if (health.kind === 'loading') return { reason: '本机健康读取中。' };
  if (health.kind === 'error')
    return { reason: `本机健康读取失败：${health.message}` };
  const section = health.report.sections.find((entry) => entry.code === code);
  return section ? { section } : { reason: '健康报告未提供' };
}

/** 从健康原文里摘出「<词> N 秒」的 N；原文不是这个说法时返回 null，不猜。 */
function seconds(detail: string, word: string): string | null {
  const match = new RegExp(`${word}\\s*(\\d+(?:\\.\\d+)?)\\s*秒`).exec(detail);
  return match ? `${match[1]} S` : null;
}

function syncCell(
  health: HealthStatus,
  label: string,
  word: string,
  signalWhenDegraded: boolean,
): TitleBlockCell {
  const found = healthSection(health, HEALTH_SYNC);
  if ('reason' in found) return missing(label, found.reason);
  const { section } = found;
  const original = `${section.title}：${section.detail}`;
  const value = seconds(section.detail, word);
  if (value === null)
    return missing(label, `健康报告没有给出${label}的秒数。${original}`);
  const signal = signalWhenDegraded && section.state === 'degraded';
  return signal
    ? { label, value, tone: 'signal', reason: original }
    : { label, value, reason: original };
}

function sessionCell(health: HealthStatus): TitleBlockCell {
  const label = '会话';
  const found = healthSection(health, HEALTH_SESSION);
  if ('reason' in found) return missing(label, found.reason);
  const { section } = found;
  const first = section.detail.split(/[，,；;。]/)[0]?.trim();
  if (!first) return missing(label, '健康报告的会话一节没有正文。');
  return section.state === 'degraded'
    ? { label, value: first, tone: 'signal', reason: section.detail }
    : { label, value: first, reason: section.detail };
}

/** 黑板相关两格：未结束条目数、尝试仍在进行的条目数。 */
function boardCell(
  overview: OverviewState,
  label: string,
  count: (state: Extract<OverviewState, { kind: 'ready' }>) => number,
): TitleBlockCell {
  switch (overview.kind) {
    case 'loading':
      return missing(label, '黑板汇总读取中。');
    case 'disabled':
      return missing(label, '黑板未启用');
    case 'error':
      return missing(label, `黑板汇总读取失败：${overview.message}`);
    case 'ready': {
      const value = count(overview);
      return overview.overview.itemsTruncated
        ? {
            label,
            value: `${counts.format(value)}+`,
            reason: '黑板汇总已截断（最多 500 条），实际可能更多。',
          }
        : { label, value: counts.format(value) };
    }
  }
}

function rollupCell(
  label: string,
  rollup: CountRollup,
  what: string,
): TitleBlockCell {
  if (rollup.value !== null)
    return { label, value: counts.format(rollup.value) };
  return missing(
    label,
    rollup.unavailable.length > 0
      ? `${rollup.unavailable.length} 个项目的${what}不可得：${rollup.unavailable.join('、')}。`
      : `服务端没有给出${what}。`,
  );
}

export interface ReadingInputs {
  workbench: Workbench;
  health: HealthStatus;
  overview: OverviewState;
}

/** 十二格，顺序与规范第 6 节一致：按行从左到右排进 3 行 × 4 列。 */
export function titleCells({
  workbench,
  health,
  overview,
}: ReadingInputs): TitleBlockCell[] {
  return [
    freshnessCell(workbench.freshness),
    syncCell(health, '每轮同步', '每轮', true),
    syncCell(health, '同步时限', '时限', false),
    boardCell(
      overview,
      '黑板在跑',
      ({ overview: data }) =>
        data.items.filter((item) => !isFinished(item)).length,
    ),
    missing('agent 程序', '接口未提供 agent 程序版本。'),
    missing('人侧发行', '接口未提供人侧发行号。'),
    sessionCell(health),
    boardCell(
      overview,
      '执行',
      ({ overview: data }) =>
        data.items.filter((item) => item.attempt?.active === true).length,
    ),
    { label: '可读项目', value: counts.format(workbench.projects.length) },
    rollupCell('副本文件', workbench.counts.documents, '文档数'),
    missing('副本大小', '接口未提供副本大小。'),
    rollupCell('未关闭 BUG', workbench.counts.open_bugs, '未关闭 BUG 数'),
  ];
}
