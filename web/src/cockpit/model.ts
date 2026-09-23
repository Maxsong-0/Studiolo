/**
 * 驾驶舱共用的小模型（T-130 设计规范第 6 节）：黑板汇总的读取状态、「等你」的判定与项目名。
 * 只做判定与换算，不补数据：拿不到的一律交给调用方显示「—」并写明原因。
 */
import type {
  BoardOverview,
  BoardOverviewItem,
  BoardOverviewStatus,
} from '../boardOverviewModel';
import type { Project } from '../types';

/** 黑板汇总的读取状态。404（本机未启用黑板）单列为 disabled，其余失败一律 error。 */
export type OverviewState =
  | { kind: 'loading' }
  | { kind: 'ready'; overview: BoardOverview }
  | { kind: 'disabled' }
  | { kind: 'error'; message: string };

/** 驾驶舱取的时间窗：默认 24 小时，足以盖住「今天」。 */
export const COCKPIT_WINDOW_HOURS = 24;

export const HOUR_MS = 3_600_000;
export const DAY_MS = 24 * HOUR_MS;

/** 等主人动手的三种任务状态，与类型标签一一对应。 */
const waitingKinds: Partial<Record<BoardOverviewStatus, string>> = {
  needs_review: '待验收',
  needs_revision: '待修订',
  needs_reconfirm: '待再次确认',
};

/** 已结束的四种状态；`approved` 与各 `needs_*` 仍算未结束（与契约第 3 节一致）。 */
const finished: readonly BoardOverviewStatus[] = [
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
];

export function isFinished(item: BoardOverviewItem): boolean {
  return finished.includes(item.status);
}

/** 等你的条目返回它的类型标签（待验收／待修订／待再次确认），其余返回 null。 */
export function waitingKind(item: BoardOverviewItem): string | null {
  return waitingKinds[item.status] ?? null;
}

/**
 * 黑板条目所属项目的显示名：工作台里有同一 root 的项目就用它的名字，
 * 否则退回路径最后一段（黑板项目路径已按 VaultPath 校验，没有空段）。
 */
export function projectName(
  path: string,
  projects: readonly Project[],
): string {
  const known = projects.find((project) => project.root === path);
  if (known) return known.name;
  const segments = path.split('/');
  return segments[segments.length - 1] || path;
}

/** 汇总生成时刻（毫秒）。黑板相关的「距今」「今天」一律以服务端这一刻为准，不掺本机时钟。 */
export function overviewNow(overview: BoardOverview): number {
  return Date.parse(overview.generatedAt);
}
