/*
 * 「今天的时间线」：由汇总接口的事件推导按执行者分的泳道（纯函数）。
 * 黑块＝AI 工作：同一执行者相邻事件间隔不超过 90 分钟的并成一段；
 * 红块＝你的审批：主人的动作（`execution.*`，主体只有任务、没有 attempt）单列一条「你」。
 * 事件按任务当前（或最近一次）持有者归到执行者；拿不到任务的事件只在截断时出现，不猜归属。
 */
import type {
  BoardOverviewEvent,
  BoardOverviewItem,
} from '../boardOverviewModel';
import type { Lane, LaneBlock, LaneTone } from '../ui';
import {
  executorLabels,
  executorOf,
  groupByExecutor,
} from './executorGrouping';

export const WORK_GAP_MS = 90 * 60_000;
/** 主人的一次动作常是「意图＋结果」两条，相隔几秒：5 分钟内的并成一块。 */
export const OWNER_GAP_MS = 5 * 60_000;
export const OWNER_LANE = '你';
const HOUR_MS = 3_600_000;
/** 画幅至少覆盖 08:00–20:00（画布主稿的一天），有更早或更晚的事件再往外扩。 */
const DAY_FROM = 8;
const DAY_TO = 20;
const STEPS = [1, 2, 3, 4, 6];
const MAX_INTERVALS = 5;

export interface TodayLanes {
  /** 轨道范围（毫秒时间戳，本地时间整点）。 */
  from: number;
  to: number;
  ticks: string[];
  lanes: Lane[];
  /** 今天落在窗口里的事件条数。 */
  events: number;
}

export const isOwnerAction = (action: string) =>
  action.startsWith('execution.');

function merge(points: number[], gap: number, tone: LaneTone): LaneBlock[] {
  const blocks: LaneBlock[] = [];
  for (const point of [...points].sort((left, right) => left - right)) {
    const last = blocks[blocks.length - 1];
    if (last && point - last.end <= gap) last.end = point;
    else blocks.push({ start: point, end: point, tone });
  }
  return blocks;
}

const clock = (hour: number) => `${String(hour).padStart(2, '0')}:00`;

/**
 * 今天（`generatedAt` 所在的本地日，零点到 generatedAt）的泳道；今天没有事件时返回 null。
 * 泳道顺序与拓扑的执行者组序一致，只列今天有动静的执行者，「你」在最后。
 */
export function todayLanes(
  items: readonly BoardOverviewItem[],
  events: readonly BoardOverviewEvent[],
  generatedAt: number,
): TodayLanes | null {
  const day = new Date(generatedAt);
  const dayStart = new Date(
    day.getFullYear(),
    day.getMonth(),
    day.getDate(),
  ).getTime();
  const today = events
    .map((event) => ({ event, at: Date.parse(event.at) }))
    .filter(({ at }) => at >= dayStart && at <= generatedAt);
  if (!today.length) return null;

  const byId = new Map(items.map((item) => [item.id, item]));
  const labels = executorLabels(items.map(executorOf));
  const work = new Map<string, number[]>();
  const owner: number[] = [];
  for (const { event, at } of today) {
    if (isOwnerAction(event.action)) {
      owner.push(at);
      continue;
    }
    const item = byId.get(event.taskId);
    if (!item) continue;
    const key = executorOf(item).key;
    const points = work.get(key);
    if (points) points.push(at);
    else work.set(key, [at]);
  }
  const lanes: Lane[] = [];
  for (const group of groupByExecutor(items, labels)) {
    const points = work.get(group.key);
    if (points?.length)
      lanes.push({
        name: group.label,
        blocks: merge(points, WORK_GAP_MS, 'work'),
      });
  }
  if (owner.length)
    lanes.push({
      name: OWNER_LANE,
      blocks: merge(owner, OWNER_GAP_MS, 'owner'),
    });
  if (!lanes.length) return null;

  const times = today.map(({ at }) => at);
  const firstHour = Math.floor((Math.min(...times) - dayStart) / HOUR_MS);
  const lastHour = Math.ceil((Math.max(...times) - dayStart) / HOUR_MS);
  let fromHour = Math.min(DAY_FROM, firstHour);
  const span = Math.min(24, Math.max(DAY_TO, lastHour)) - fromHour;
  const step =
    STEPS.find((value) => Math.ceil(span / value) <= MAX_INTERVALS) ?? 6;
  const intervals = Math.ceil(span / step);
  if (fromHour + intervals * step > 24)
    fromHour = Math.max(0, 24 - intervals * step);
  return {
    from: dayStart + fromHour * HOUR_MS,
    to: dayStart + (fromHour + intervals * step) * HOUR_MS,
    ticks: Array.from({ length: intervals + 1 }, (_, index) =>
      clock(fromHour + index * step),
    ),
    lanes,
    events: today.length,
  };
}
