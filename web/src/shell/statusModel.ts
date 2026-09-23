import type { Freshness } from '../freshness';
import type { HealthStatus } from './useHealthStatus';

/** normal＝实心方块「正常」；pending＝空心方块（读取或核验在途）；problem＝红色方括号。 */
export type StatusLevel = 'normal' | 'pending' | 'problem';

export interface StatusSummary {
  level: StatusLevel;
  /** 胶囊上的简短标签：全部正常时为「正常」，否则是最严重的那一项。 */
  label: string;
}

export interface StatusInputs {
  /** 是否已有工作台快照。 */
  loaded: boolean;
  freshness: Freshness | null | undefined;
  /** 工作台正在读取或刷新。 */
  refreshing: boolean;
  /** 最近一次读取工作台失败。 */
  failed: boolean;
  health: HealthStatus;
  /** 数据提示条数。 */
  diagnostics: number;
}

interface Signal {
  level: StatusLevel;
  /** 越大越严重；同级取先列出的一项（新鲜度 → 健康 → 数据提示）。 */
  rank: number;
  label: string;
}

const normal: Signal = { level: 'normal', rank: 0, label: '正常' };

function problem(rank: number, label: string): Signal {
  return { level: 'problem', rank, label };
}

function pending(rank: number, label: string): Signal {
  return { level: 'pending', rank, label };
}

/**
 * 陈旧原因的简短标签：逐条取自 freshness.ts 中该原因的原文，只截成胶囊放得下的长度；
 * 完整原文仍在浮层的新鲜度一栏。未收录的新原因退回「快照陈旧」，不猜含义。
 */
const staleLabels: Record<string, string> = {
  initializing: '首次快照准备中',
  refreshing: '服务核验中',
  periodic_check: '周期核验中',
  changed: '内容已变化',
  watcher_rescan: '目录待重扫',
  watcher_failed: '目录监视失效',
  capture_failed: '内容读取未完成',
  build_failed: '快照构建失败',
  binding_changed: '目录绑定变化',
  unresolved: '写入结果未决',
  writer_state_invalid: '写者状态不明',
  stopping: '服务停止中',
  unavailable: '状态暂不可确认',
};

/** 与 freshnessReason 同一判断顺序：先看服务端给的原因，再看未决写入与目录监视。 */
function staleLabel(freshness: Freshness): string {
  if (freshness.reason)
    return Object.hasOwn(staleLabels, freshness.reason)
      ? staleLabels[freshness.reason]
      : '快照陈旧';
  if (freshness.unresolved) return staleLabels.unresolved;
  if (!freshness.watcher_healthy) return staleLabels.watcher_failed;
  return '快照未追平';
}

/** 与 FreshnessNotice 同一判断顺序：在途与失败优先，其次才看新鲜度证据本身。 */
function freshnessSignal(inputs: StatusInputs): Signal {
  const { loaded, freshness, refreshing, failed } = inputs;
  if (!loaded)
    return failed && !refreshing
      ? problem(100, '无法读取工作台')
      : pending(20, '读取中');
  if (refreshing) return pending(20, '核验中');
  if (failed) return problem(90, '核验失败');
  if (!freshness) return problem(50, '无新鲜度证据');
  if (freshness.stale) return problem(70, staleLabel(freshness));
  return normal;
}

/** 未知不等于正常：读取失败、整体未知、证据不全都算问题，只有证据完整的正常才算正常。 */
function healthSignal(health: HealthStatus): Signal {
  if (health.kind === 'loading') return pending(10, '读取健康');
  if (health.kind === 'error') return problem(60, '健康未知');
  const { report } = health;
  if (
    report.overall === 'degraded' ||
    report.sections.some((section) => section.state === 'degraded')
  )
    return problem(80, '健康降级');
  if (report.overall === 'unknown') return problem(60, '健康未知');
  if (!report.evidenceComplete) return problem(55, '健康证据不全');
  return normal;
}

function diagnosticsSignal(count: number): Signal {
  return count > 0 ? problem(40, `数据提示 ${count}`) : normal;
}

/** 把新鲜度、本机健康与数据提示收成胶囊上的一句：全部正常才说正常，否则报最严重的一项。 */
export function summarizeStatus(inputs: StatusInputs): StatusSummary {
  const signals = [
    freshnessSignal(inputs),
    healthSignal(inputs.health),
    diagnosticsSignal(inputs.diagnostics),
  ];
  const worst = signals.reduce((current, next) =>
    next.rank > current.rank ? next : current,
  );
  return { level: worst.level, label: worst.label };
}
