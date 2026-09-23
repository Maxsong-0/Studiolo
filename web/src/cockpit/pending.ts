/**
 * 待我动作（T-130 设计规范第 6 节第 1、2、3 条）：黑板里等你的任务＋工作台待办，统一编号 1..N。
 *
 * - 黑板条目（needs_review／needs_revision／needs_reconfirm）排在前面，最久没动静的先列；
 *   画进拓扑缩略的那几条圈号红色实心，并生成同号标记；缩略图放不下的退成空心圈。
 * - 工作台待办（Action）排在后面，按服务端给的顺序，圈号黑色空心，点开是原文。
 * - 拓扑缩略最多画 6 条未结束任务：先保证等你的条目上图，再补尝试仍在进行的，最后补其余；
 *   其余条数交给「另 N 条，去协作页」。
 */
import type { BoardOverviewItem } from '../boardOverviewModel';
import type { TopologyMarker } from '../collab/TopologyDiagram';
import type { Action, Project } from '../types';
import {
  isFinished,
  overviewNow,
  projectName,
  waitingKind,
  type OverviewState,
} from './model';

/** 驾驶舱拓扑缩略最多画几条任务（规范第 6 节第 2 条）。 */
export const MINI_TASKS = 6;

export interface BoardPending {
  source: 'board';
  n: number;
  item: BoardOverviewItem;
  /** 类型标签：待验收／待修订／待再次确认。 */
  kind: string;
  project: string;
  /** 已画进拓扑缩略、图上有同号红圈。 */
  marked: boolean;
}

export interface InboxPending {
  source: 'inbox';
  n: number;
  action: Action;
  /** 类型标签：取工作台待办的 kind 原文。 */
  kind: string;
  project: string;
}

export type PendingEntry = BoardPending | InboxPending;

export interface PendingPlan {
  entries: PendingEntry[];
  /** 待我动作总数；黑板还在读或读取失败时，等你的黑板任务数不可知，总数为 null。 */
  total: number | null;
  /** 带时间的等你条目中最久的一条已等了多少毫秒；没有带时间的条目时为 null。 */
  oldestWaitMs: number | null;
  /** 黑板汇总是否被截断（此时等你的任务可能没列全）。 */
  truncated: boolean;
  /** 未结束的黑板任务条数；黑板状态未定时为 null。 */
  unfinished: number | null;
  /** 画进拓扑缩略的条目，保持汇总原顺序。 */
  miniItems: BoardOverviewItem[];
  /** 未结束但没画进缩略图的条数。 */
  omitted: number;
  /** 缩略图上的红圈：只来自黑板条目。 */
  markers: TopologyMarker[];
}

/** 等你条目的起始时间：该尝试最后一次进展；没有进展记录（或记录为 0）时不算带时间。 */
function waitingSince(item: BoardOverviewItem): number | null {
  const last = item.progress?.lastProgressMs ?? null;
  return last !== null && last > 0 ? last : null;
}

/** 最久没动静的排前面；没有时间的排在带时间的后面；其余保持汇总原顺序（项目、id）。 */
function byWaitingAge(
  left: { item: BoardOverviewItem; index: number },
  right: { item: BoardOverviewItem; index: number },
): number {
  const a = waitingSince(left.item);
  const b = waitingSince(right.item);
  if (a !== null && b !== null && a !== b) return a - b;
  if (a === null && b !== null) return 1;
  if (a !== null && b === null) return -1;
  return left.index - right.index;
}

/**
 * 缩略图选条：等你的条目按待办顺序先上，再补尝试仍在进行的，最后补其余未结束的，
 * 同一档保持汇总原顺序。返回被选中的 id 集合。
 */
function pickMini(
  waiting: readonly BoardOverviewItem[],
  unfinished: readonly BoardOverviewItem[],
  max: number,
): Set<string> {
  const order = [
    ...waiting,
    ...unfinished.filter(
      (item) => !waitingKind(item) && item.attempt?.active === true,
    ),
    ...unfinished.filter(
      (item) => !waitingKind(item) && item.attempt?.active !== true,
    ),
  ];
  return new Set(order.slice(0, Math.max(0, max)).map((item) => item.id));
}

export function planPending(
  state: OverviewState,
  inbox: readonly Action[],
  projects: readonly Project[],
  maxMini = MINI_TASKS,
): PendingPlan {
  const overview = state.kind === 'ready' ? state.overview : null;
  const unfinishedItems = overview
    ? overview.items.filter((item) => !isFinished(item))
    : [];
  const waiting = unfinishedItems
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => waitingKind(item) !== null)
    .sort(byWaitingAge)
    .map(({ item }) => item);
  const mini = pickMini(waiting, unfinishedItems, maxMini);

  const entries: PendingEntry[] = [];
  const markers: TopologyMarker[] = [];
  for (const item of waiting) {
    const n = entries.length + 1;
    const marked = mini.has(item.id);
    entries.push({
      source: 'board',
      n,
      item,
      kind: waitingKind(item) ?? '',
      project: projectName(item.project, projects),
      marked,
    });
    if (marked) markers.push({ n, target: { taskId: item.id } });
  }
  for (const action of inbox) {
    entries.push({
      source: 'inbox',
      n: entries.length + 1,
      action,
      kind: action.kind,
      project: action.project_name,
    });
  }

  let oldestWaitMs: number | null = null;
  if (overview) {
    const now = overviewNow(overview);
    for (const item of waiting) {
      const since = waitingSince(item);
      if (since === null) continue;
      const age = Math.max(0, now - since);
      oldestWaitMs = oldestWaitMs === null ? age : Math.max(oldestWaitMs, age);
    }
  }

  // 黑板未启用：本机根本没有黑板，等你的黑板任务确实是 0；读取中或失败则总数不可知。
  const settled = state.kind === 'ready' || state.kind === 'disabled';
  const miniItems = unfinishedItems.filter((item) => mini.has(item.id));
  return {
    entries,
    total: settled ? entries.length : null,
    oldestWaitMs,
    truncated: overview?.itemsTruncated ?? false,
    unfinished: overview ? unfinishedItems.length : null,
    miniItems,
    omitted: unfinishedItems.length - miniItems.length,
    markers,
  };
}

/** 工作台待办标题常以「类型：」开头，类型已单独成列，列表里去掉这段重复。 */
export function actionTitle(action: Action): string {
  return action.title.startsWith(`${action.kind}：`)
    ? action.title.slice(action.kind.length + 1)
    : action.title;
}
