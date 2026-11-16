/**
 * 「自上次查看以来」（T-195；设计规范第 6 节）：四个数，全部从已经在手的三份读数里数出来（纯函数）。
 *
 * - 任务完成：黑板里现在是「已完成」、并且起点之后还有事件的任务。
 * - 新建任务：起点之后的 `task.create` 结果事件。
 * - 新提案和草案：起点之后提交的策略草案，不论现在什么状态。
 * - 写入：提交水位前进了多少，即经工作台写进去的次数（网页编辑、文件操作、AI 写回）；
 *   在别处直接改文件不经这条水位，不算在内。
 *
 * 起点是上次离开时看到的数据时刻（见 lastSeen.ts）；这台浏览器还没有记录时，先按最近 24 小时算，并照实说明。
 * 黑板汇总只带最近 24 小时的事件：起点更早时，黑板那两个数只含这 24 小时，同样照实说明。
 * 拿不到的数是 null（显示「—」），原因写进 reason，不编数。
 */
import type { BoardOverview } from '../boardOverviewModel';
import { clockLabel } from '../collab/todayLanes';
import type { Freshness } from '../freshness';
import type { SeenMark } from './lastSeen';
import {
  DAY_MS,
  overviewNow,
  type OverviewState,
  type PolicyPendingState,
} from './model';
import { waitLabel } from './wait';

export interface SinceStat {
  key: 'done' | 'created' | 'drafts' | 'writes';
  /** 数；拿不到时为 null。 */
  value: number | null;
  /** 实际可能更多（黑板事件被截断，或起点早于黑板汇总的时间窗）。 */
  atLeast: boolean;
  /** 数字后面的说法，如「个任务完成」。 */
  unit: string;
  /** 数字下面的等宽小字：是哪几条，或这个数怎么来的。 */
  note: string;
  /** 拿不到时的原因。 */
  reason?: string;
}

export interface Since {
  /** 这台浏览器有上次查看的记录；没有时四个数按最近 24 小时算。 */
  known: boolean;
  from: number;
  to: number;
  /** 标题行右侧的说明：从什么时候起、隔了多久。 */
  caption: string;
  stats: SinceStat[];
}

/** 说明里最多点名几条任务。 */
const NAMED = 3;
/** 没有编号前缀的标题最多留几个字。 */
const TITLE_MAX = 10;

/** 标题的短写：以「T-021」这类编号开头就只留编号，否则截前几个字。 */
function shortTitle(title: string): string {
  const numbered = /^[A-Za-z]+-\d+/.exec(title.trim());
  if (numbered) return numbered[0];
  const characters = Array.from(title.trim());
  return characters.length > TITLE_MAX
    ? `${characters.slice(0, TITLE_MAX).join('')}…`
    : characters.join('');
}

function named(titles: readonly string[]): string {
  const shown = titles.slice(0, NAMED).map(shortTitle).join(' · ');
  return titles.length > NAMED ? `${shown} 等 ${titles.length} 条` : shown;
}

/** 「昨天 22:40」「08:05」「11月10日 09:30」：与 `now` 同一天只写钟点。 */
function dayClock(ms: number, now: number): string {
  const day = new Date(ms);
  const today = new Date(now);
  const midnight = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((midnight(today) - midnight(day)) / DAY_MS);
  if (days === 0) return clockLabel(ms);
  if (days === 1) return `昨天 ${clockLabel(ms)}`;
  return `${day.getMonth() + 1}月${day.getDate()}日 ${clockLabel(ms)}`;
}

function boardStats(
  board: OverviewState,
  from: number,
): { done: SinceStat; created: SinceStat; clipped: boolean } {
  const units = { done: '个任务完成', created: '个新建任务' } as const;
  const missing = (reason: string) => ({
    done: {
      key: 'done' as const,
      value: null,
      atLeast: false,
      unit: units.done,
      note: '',
      reason,
    },
    created: {
      key: 'created' as const,
      value: null,
      atLeast: false,
      unit: units.created,
      note: '',
      reason,
    },
    clipped: false,
  });
  if (board.kind === 'loading') return missing('黑板汇总读取中。');
  if (board.kind === 'disabled') return missing('黑板未启用。');
  if (board.kind === 'error')
    return missing(`黑板汇总读取失败：${board.message}`);

  const overview: BoardOverview = board.overview;
  const clipped = from < Date.parse(overview.windowFrom);
  const atLeast = clipped || overview.eventsTruncated;
  const titles = new Map(overview.items.map((item) => [item.id, item.title]));
  const latest = new Map<string, number>();
  const createdTitles: string[] = [];
  for (const event of overview.events) {
    const ms = Date.parse(event.at);
    if (ms <= from) continue;
    latest.set(event.taskId, ms);
    if (event.action === 'task.create' && event.phase === 'outcome')
      createdTitles.push(titles.get(event.taskId) ?? event.taskId);
  }
  const doneTitles = overview.items
    .filter((item) => item.status === 'succeeded' && latest.has(item.id))
    .map((item) => item.title);
  const stat = (
    key: 'done' | 'created',
    list: readonly string[],
    empty: string,
  ): SinceStat => ({
    key,
    value: list.length,
    atLeast,
    unit: units[key],
    note: list.length > 0 ? named(list) : empty,
  });
  return {
    done: stat('done', doneTitles, '黑板上没有新完成的'),
    created: stat('created', createdTitles, '黑板上没有新建的'),
    clipped,
  };
}

function draftStat(policy: PolicyPendingState, from: number): SinceStat {
  const base = {
    key: 'drafts' as const,
    atLeast: false,
    unit: '份新提案和草案',
  };
  const missing = (reason: string): SinceStat => ({
    ...base,
    value: null,
    note: '',
    reason,
  });
  if (policy.kind === 'loading') return missing('策略草案读取中。');
  if (policy.kind === 'absent') return missing('本入口没有接入策略平面。');
  if (policy.kind === 'error')
    return missing(`策略草案读取失败：${policy.message}`);
  const fresh = policy.desired.filter(
    (entry) => Date.parse(entry.submitted_at) > from,
  );
  const proposals = fresh.filter(
    (entry) => entry.submitter_kind === 'agent',
  ).length;
  return {
    ...base,
    value: fresh.length,
    note:
      fresh.length === 0
        ? '没有新提交的'
        : proposals === 0
          ? '都是你自己交的'
          : `其中 ${proposals} 份是 AI 提的`,
  };
}

function writeStat(
  baseline: SeenMark | null,
  freshness: Freshness | null | undefined,
): SinceStat {
  const base = { key: 'writes' as const, atLeast: false, unit: '次写入' };
  const missing = (reason: string): SinceStat => ({
    ...base,
    value: null,
    note: '',
    reason,
  });
  if (!baseline) return missing('没有上次查看时的提交水位，没法比。');
  if (baseline.committed === null)
    return missing('上次查看时工作台快照没有给出提交水位。');
  const current = freshness?.committed_through ?? null;
  if (current === null) return missing('工作台快照没有给出提交水位。');
  // 水位是 u64 的十进制串，用 BigInt 相减，免得超过 2^53 失真。
  const advanced = BigInt(current) - BigInt(baseline.committed);
  if (advanced < 0n) return missing('提交水位比上次查看时还小，没法比。');
  return advanced > BigInt(Number.MAX_SAFE_INTEGER)
    ? missing('提交水位差得太大，没法显示。')
    : { ...base, value: Number(advanced), note: '网页编辑和 AI 写回' };
}

export interface SinceInputs {
  /** 上次离开时的记号；这台浏览器还没有记录时为 null。 */
  baseline: SeenMark | null;
  board: OverviewState;
  policy: PolicyPendingState;
  freshness: Freshness | null | undefined;
  /** 本机此刻：只在黑板没读到时当「现在」用。 */
  now: number;
}

export function sinceLastSeen({
  baseline,
  board,
  policy,
  freshness,
  now,
}: SinceInputs): Since {
  const to = board.kind === 'ready' ? overviewNow(board.overview) : now;
  // 记号比现在的读数还新（刚离开又回来、读数还是旧的）：起点收到「现在」，四个数都是 0。
  const from = Math.min(baseline?.at ?? to - DAY_MS, to);
  const { done, created, clipped } = boardStats(board, from);
  const caption = baseline
    ? `${dayClock(from, to)} 起，${waitLabel(to - from)}${
        clipped ? '；黑板那两个数只含最近 24 小时' : ''
      }`
    : '这台浏览器还没有上次查看的记录，先按最近 24 小时算';
  return {
    known: baseline !== null,
    from,
    to,
    caption,
    stats: [
      done,
      created,
      draftStat(policy, from),
      writeStat(baseline, freshness),
    ],
  };
}
