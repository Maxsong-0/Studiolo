/**
 * 「等你」的队列（T-195；设计规范第 6 节）：三路并成一列，统一编号 1..N。
 *
 * - 黑板里等你的任务（needs_review／needs_revision／needs_reconfirm），从它最后一次进展算起；
 * - 策略里等你的提案和草案（待复核、已校验可发布；被同一提交者更新的草案挡住的较早那份不算，
 *   要处理的是更新的那份），从提交那一刻算起；
 * - 工作台待办（Action），接口不带时间。
 *
 * 带时间的排前面，等得最久的在最上；不带时间的接在后面，先黑板、再策略、最后工作台待办，各按来源原顺序。
 * 总数只由 {@link pendingCount} 算：驾驶舱巨号与顶栏「驾驶舱 N」角标同一口径（T-162），
 * 任何一路还在读或读失败时总数不可知，不拿另外两路冒充。
 */
import type { BoardOverview, BoardOverviewItem } from '../boardOverviewModel';
import type { ListedDesired } from '../policyModel';
import type { Action, Project } from '../types';
import {
  isFinished,
  overviewNow,
  projectName,
  waitingKind,
  type OverviewState,
  type PolicyPendingState,
} from './model';

/** 策略一路的两种类型标签：AI 交来的提案，和你自己交的、还没发布的草案。 */
export const KIND_PROPOSAL = 'AI 提案';
export const KIND_DRAFT = '待发布草案';

interface Timed {
  /** 从哪一刻开始等（毫秒时间戳）；来源不带时间时为 null。 */
  since: number | null;
  /** 已等了多久（毫秒，不为负）；没有起点时为 null。 */
  waitMs: number | null;
}

export interface BoardPending extends Timed {
  source: 'board';
  n: number;
  item: BoardOverviewItem;
  /** 类型标签：待验收／待修订／待再次确认。 */
  kind: string;
  project: string;
}

export interface PolicyPending extends Timed {
  source: 'policy';
  n: number;
  desired: ListedDesired;
  /** 类型标签：{@link KIND_PROPOSAL}／{@link KIND_DRAFT}。 */
  kind: string;
}

export interface InboxPending extends Timed {
  source: 'inbox';
  n: number;
  action: Action;
  /** 类型标签：取工作台待办的 kind 原文；原文为空时写「未知」。 */
  kind: string;
  project: string;
}

export type PendingEntry = BoardPending | PolicyPending | InboxPending;

export interface PendingPlan {
  entries: PendingEntry[];
  /** 等你的总数，即 {@link pendingCount}；任何一路还在读或读失败时为 null。 */
  total: number | null;
  /** 带时间的条目里等得最久的那条已等了多少毫秒；没有带时间的条目时为 null。 */
  oldestWaitMs: number | null;
  /** 类型分布：每种类型几条，按在队列里第一次出现的先后。 */
  kinds: { kind: string; count: number }[];
  /** 黑板汇总是否被截断（此时等你的任务可能没列全）。 */
  truncated: boolean;
}

/** 黑板里等你的任务：未结束、状态是待验收／待修订／待再次确认的条目，保持汇总原顺序。 */
function waitingItems(overview: BoardOverview): BoardOverviewItem[] {
  return overview.items.filter(
    (item) => !isFinished(item) && waitingKind(item) !== null,
  );
}

/**
 * 策略里等你的草案：待复核或已校验，并且没有被同一提交者更新的草案挡住（接口给的 `blocked_by` 为空）。
 * 校验未通过的不算：它要先改好重交，不是等你点头。保持接口原顺序。
 */
export function waitingDesired(
  desired: readonly ListedDesired[],
): ListedDesired[] {
  return desired.filter(
    (entry) =>
      (entry.status === 'pending' || entry.status === 'validated') &&
      entry.blocked_by.length === 0,
  );
}

/**
 * 等你的总数：工作台待办＋黑板里等你的任务＋策略里等你的提案和草案（T-162、T-195）。
 *
 * 驾驶舱巨号与顶栏「驾驶舱 N」角标都只用它算，而且喂同一份工作台快照、同一次黑板汇总读取、
 * 同一次策略草案读取，所以两处任何时刻都相等。
 * - 还没有工作台快照：null（显示「—」）。
 * - 黑板未启用（404）或本入口没装策略平面（404 `policy_plane_absent`）：那一路确实是 0。
 * - 黑板或策略任何一路还在读、读失败：那一路的条数不可知，总数为 null（显示「—」），不拿其余的冒充总数。
 */
export function pendingCount(
  workbench: { readonly inbox: readonly Action[] } | null,
  board: OverviewState,
  policy: PolicyPendingState,
): number | null {
  if (!workbench) return null;
  if (board.kind === 'loading' || board.kind === 'error') return null;
  if (policy.kind === 'loading' || policy.kind === 'error') return null;
  return (
    workbench.inbox.length +
    (board.kind === 'ready' ? waitingItems(board.overview).length : 0) +
    (policy.kind === 'ready' ? waitingDesired(policy.desired).length : 0)
  );
}

/** 等你的黑板任务从哪一刻算：该尝试最后一次进展；没有进展记录（或记录为 0）时不算带时间。 */
function boardSince(item: BoardOverviewItem): number | null {
  const last = item.progress?.lastProgressMs ?? null;
  return last !== null && last > 0 ? last : null;
}

/** 草案从提交那一刻算；时间串解析不出来时不算带时间。 */
function desiredSince(desired: ListedDesired): number | null {
  const ms = Date.parse(desired.submitted_at);
  return Number.isFinite(ms) && ms > 0 ? ms : null;
}

const waited = (now: number, since: number | null) =>
  since === null ? null : Math.max(0, now - since);

type Unnumbered =
  Omit<BoardPending, 'n'> | Omit<PolicyPending, 'n'> | Omit<InboxPending, 'n'>;

export interface PendingSources {
  board: OverviewState;
  policy: PolicyPendingState;
  inbox: readonly Action[];
  projects: readonly Project[];
}

export function planPending({
  board,
  policy,
  inbox,
  projects,
}: PendingSources): PendingPlan {
  const overview = board.kind === 'ready' ? board.overview : null;
  const rows: Unnumbered[] = [];
  if (overview) {
    const now = overviewNow(overview);
    for (const item of waitingItems(overview)) {
      const since = boardSince(item);
      rows.push({
        source: 'board',
        item,
        kind: waitingKind(item) ?? '',
        project: projectName(item.project, projects),
        since,
        waitMs: waited(now, since),
      });
    }
  }
  if (policy.kind === 'ready')
    for (const desired of waitingDesired(policy.desired)) {
      const since = desiredSince(desired);
      rows.push({
        source: 'policy',
        desired,
        kind: desired.submitter_kind === 'agent' ? KIND_PROPOSAL : KIND_DRAFT,
        since,
        waitMs: waited(policy.readAt, since),
      });
    }
  for (const action of inbox)
    rows.push({
      source: 'inbox',
      action,
      kind: action.kind || '未知',
      project: action.project_name,
      since: null,
      waitMs: null,
    });

  // 越早开始等的越靠前；不带时间的排在带时间的后面；其余保持上面的先后（黑板、策略、工作台待办）。
  const ordered = rows
    .map((row, index) => ({ row, index }))
    .sort((left, right) => {
      const a = left.row.since;
      const b = right.row.since;
      if (a !== null && b !== null && a !== b) return a - b;
      if (a === null && b !== null) return 1;
      if (a !== null && b === null) return -1;
      return left.index - right.index;
    });
  const entries = ordered.map(
    ({ row }, index) => ({ ...row, n: index + 1 }) as PendingEntry,
  );

  let oldestWaitMs: number | null = null;
  const tally = new Map<string, number>();
  for (const entry of entries) {
    if (entry.waitMs !== null)
      oldestWaitMs =
        oldestWaitMs === null
          ? entry.waitMs
          : Math.max(oldestWaitMs, entry.waitMs);
    tally.set(entry.kind, (tally.get(entry.kind) ?? 0) + 1);
  }

  return {
    entries,
    // 与顶栏角标同一个函数：三路都读完（或确实没有）时恰是上面列出的条数，否则不可知。
    total: pendingCount({ inbox }, board, policy),
    oldestWaitMs,
    kinds: [...tally].map(([kind, count]) => ({ kind, count })),
    truncated: overview?.itemsTruncated ?? false,
  };
}

/** 队列里一行的稳定键：重排、重读之后选中的还是同一件事。 */
export function entryKey(entry: PendingEntry): string {
  switch (entry.source) {
    case 'board':
      return `board:${entry.item.id}`;
    case 'policy':
      return `policy:${entry.desired.id}`;
    case 'inbox':
      return `inbox:${entry.action.id}`;
  }
}

/** 工作台待办标题常以「类型：」开头，类型已单独成列，列表里去掉这段重复。 */
export function actionTitle(action: Action): string {
  return action.title.startsWith(`${action.kind}：`)
    ? action.title.slice(action.kind.length + 1)
    : action.title;
}
