/**
 * 今天的时间线（T-130 设计规范第 6 节第 4 条）：按执行者分泳道，黑块＝AI 工作，红块＝你的审批；
 * 下接最近三条动态。全部由黑板汇总的 events 推出，不补任何数据。
 *
 * - 时间范围：汇总生成时刻向上取整到整点为终点，至少画最近 12 小时；今天更早就有可画事件时，
 *   从今天第一条所在的整点画起。
 * - AI 工作：只认执行体自己的活动事件（actions.ts 的 WORK_ACTIONS）；同一任务相邻两条事件之间算一段工作，
 *   相隔超过 {@link MAX_WORK_GAP_MS} 视为中间停过、断成两段；孤立的一条画成最窄块。
 * - 你的审批：`execution.*` 是主人经执行接口做的决定（启动、取消、审阅、落地），intent 到 outcome 为一块。
 */
import type { Lane, LaneBlock } from '../ui';
import type { BoardOverview, BoardOverviewItem } from '../boardOverviewModel';
import { TELEMETRY, WORK_ACTIONS, actionText, isOwnerAction } from './actions';
import { executorLabels, executorOf } from './executors';
import { HOUR_MS, overviewNow } from './model';

/** 时间线至少画最近这么久。 */
export const TIMELINE_SPAN_MS = 12 * HOUR_MS;
/** 同一任务相邻两条活动事件相隔超过这么久，就不算连续工作。 */
export const MAX_WORK_GAP_MS = 2 * HOUR_MS;
/** AI 泳道最多几条；多出的按最近活动取舍，并报出条数。 */
export const MAX_AI_LANES = 6;
/** 主人泳道的名字。 */
export const OWNER_LANE = '你';

const pad = (value: number) => String(value).padStart(2, '0');

/** 本地时刻 HH:MM。 */
export function clockLabel(ms: number): string {
  const date = new Date(ms);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export interface TimelineRange {
  from: number;
  to: number;
  /** 刻度：首尾对齐 from/to，中间均分，段数尽量让每个刻度落在整点。 */
  ticks: string[];
}

/** 刻度分几段：优先 4 段，其次挑能让每个刻度都落在整点的段数。 */
const TICK_SEGMENTS = [4, 3, 5, 6, 2];

function floorHour(ms: number): number {
  const date = new Date(ms);
  date.setMinutes(0, 0, 0);
  return date.getTime();
}

/**
 * 「今天」的范围：终点是 `now` 向上取整到整点，至少画最近 {@link TIMELINE_SPAN_MS}；
 * 今天（本地 0 点起）更早就有活动时，从今天第一条活动所在的整点画起，一天的活儿不截半。
 * 刚过 0 点时照样看得到前一晚的活动，不会因为换了日期就空着。
 */
export function timelineRange(
  now: number,
  activity: readonly number[] = [],
): TimelineRange {
  const end = new Date(now);
  if (
    end.getMinutes() !== 0 ||
    end.getSeconds() !== 0 ||
    end.getMilliseconds() !== 0
  ) {
    end.setMinutes(0, 0, 0);
    end.setHours(end.getHours() + 1);
  }
  const to = end.getTime();
  const today = new Date(now);
  const midnight = new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate(),
  ).getTime();
  let earliest: number | null = null;
  for (const ms of activity)
    if (ms >= midnight && ms <= to && (earliest === null || ms < earliest))
      earliest = ms;
  const recent = to - TIMELINE_SPAN_MS;
  const from =
    earliest === null ? recent : Math.min(recent, floorHour(earliest));
  const hours = Math.round((to - from) / HOUR_MS);
  const segments =
    TICK_SEGMENTS.find((count) => hours % count === 0) ?? TICK_SEGMENTS[0];
  const ticks = Array.from({ length: segments + 1 }, (_, index) =>
    clockLabel(from + ((to - from) * index) / segments),
  );
  return { from, to, ticks };
}

interface Interval {
  start: number;
  end: number;
}

/** 按起点排序后把重叠或首尾相接的段并起来。 */
function merge(intervals: Interval[]): Interval[] {
  const sorted = [...intervals].sort((a, b) => a.start - b.start);
  const merged: Interval[] = [];
  for (const interval of sorted) {
    const last = merged[merged.length - 1];
    if (last && interval.start <= last.end)
      last.end = Math.max(last.end, interval.end);
    else merged.push({ ...interval });
  }
  return merged;
}

/** 一个任务的活动时刻（已升序）切成工作段：相邻两条相隔不超过上限就连成一段。 */
function workSegments(times: readonly number[]): Interval[] {
  const segments: Interval[] = [];
  for (const time of times) {
    const last = segments[segments.length - 1];
    if (last && time - last.end <= MAX_WORK_GAP_MS) last.end = time;
    else segments.push({ start: time, end: time });
  }
  return segments;
}

const overlaps = (interval: Interval, range: TimelineRange) =>
  interval.start === interval.end
    ? interval.start >= range.from && interval.start <= range.to
    : interval.end > range.from && interval.start < range.to;

/**
 * 整份汇总的执行者显示名：泳道、动态与拓扑缩略同用一套，前缀长度按全部条目的执行者定，
 * 不因某条线今天没活动而变。
 */
export function overviewLabels(overview: BoardOverview): Map<string, string> {
  return executorLabels(overview.items.map(executorOf));
}

export interface TimelineLanes {
  lanes: Lane[];
  /** 范围内有活动、但超出泳道上限没画出来的执行者数。 */
  hidden: number;
}

export function timelineLanes(
  overview: BoardOverview,
  range: TimelineRange,
): TimelineLanes {
  const byId = new Map(overview.items.map((item) => [item.id, item]));
  const work = new Map<string, number[]>();
  const owner: Interval[] = [];
  const openOwner = new Map<string, number>();
  for (const event of overview.events) {
    const ms = Date.parse(event.at);
    if (isOwnerAction(event.action)) {
      const key = `${event.taskId}|${event.action}`;
      if (event.phase === 'intent') {
        const pending = openOwner.get(key);
        if (pending !== undefined) owner.push({ start: pending, end: pending });
        openOwner.set(key, ms);
      } else {
        owner.push({ start: openOwner.get(key) ?? ms, end: ms });
        openOwner.delete(key);
      }
    } else if (WORK_ACTIONS.has(event.action)) {
      const times = work.get(event.taskId);
      if (times) times.push(ms);
      else work.set(event.taskId, [ms]);
    }
  }
  for (const start of openOwner.values()) owner.push({ start, end: start });

  // 执行者 → 工作段；汇总被截断时个别事件的任务可能不在 items 里，这些没法归属，不画。
  const byExecutor = new Map<string, Interval[]>();
  for (const [taskId, times] of work) {
    const item: BoardOverviewItem | undefined = byId.get(taskId);
    if (!item) continue;
    const { key } = executorOf(item);
    const segments = byExecutor.get(key);
    if (segments) segments.push(...workSegments(times));
    else byExecutor.set(key, workSegments(times));
  }
  const labels = overviewLabels(overview);
  const active = [...byExecutor]
    .map(([key, intervals]) => ({
      key,
      intervals: merge(intervals).filter((interval) =>
        overlaps(interval, range),
      ),
    }))
    .filter((lane) => lane.intervals.length > 0);
  // 超过上限时留最近还有活动的几条，再按显示名排好。
  const kept = [...active]
    .sort(
      (a, b) =>
        Math.max(...b.intervals.map((interval) => interval.end)) -
        Math.max(...a.intervals.map((interval) => interval.end)),
    )
    .slice(0, MAX_AI_LANES)
    .map((lane) => ({ ...lane, name: labels.get(lane.key) ?? lane.key }))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));

  const toBlocks = (
    intervals: readonly Interval[],
    tone: LaneBlock['tone'],
  ): LaneBlock[] =>
    intervals.map((interval) => ({
      start: interval.start,
      end: interval.end,
      tone,
    }));
  return {
    lanes: [
      ...kept.map((lane) => ({
        name: lane.name,
        blocks: toBlocks(lane.intervals, 'work'),
      })),
      {
        name: OWNER_LANE,
        blocks: toBlocks(
          merge(owner).filter((interval) => overlaps(interval, range)),
          'owner',
        ),
      },
    ],
    hidden: active.length - kept.length,
  };
}

export interface ActivityEntry {
  key: string;
  time: string;
  who: string;
  text: string;
  /** 原始动作与阶段，放进 title 供核对。 */
  raw: string;
}

/**
 * 最近的动态：时间范围内的 outcome 事件，去掉高频遥测，按时间倒序取前几条。
 * 执行者：`execution.*` 是你；其余是该任务的执行者显示名。
 */
export function recentActivity(
  overview: BoardOverview,
  range: TimelineRange,
  count = 3,
): ActivityEntry[] {
  const byId = new Map(overview.items.map((item) => [item.id, item]));
  const labels = overviewLabels(overview);
  const entries: ActivityEntry[] = [];
  for (let index = overview.events.length - 1; index >= 0; index -= 1) {
    if (entries.length >= count) break;
    const event = overview.events[index];
    const ms = Date.parse(event.at);
    if (ms < range.from) break;
    if (
      ms > range.to ||
      event.phase !== 'outcome' ||
      TELEMETRY.has(event.action)
    )
      continue;
    const item = byId.get(event.taskId);
    entries.push({
      key: `${index}`,
      time: clockLabel(ms),
      who: isOwnerAction(event.action)
        ? OWNER_LANE
        : item
          ? (labels.get(executorOf(item).key) ?? '—')
          : '—',
      text: `${actionText(event.action)} ${item?.title ?? event.taskId}`,
      raw: `${event.action}（${event.phase}）`,
    });
  }
  return entries;
}

/**
 * 汇总生成时刻所在的「今天」：黑板相关的时刻一律以服务端为准；
 * 起点看今天最早一条能画进泳道的事件（执行体活动或你的决定）。
 */
export function overviewRange(overview: BoardOverview): TimelineRange {
  return timelineRange(
    overviewNow(overview),
    overview.events
      .filter(
        (event) =>
          WORK_ACTIONS.has(event.action) || isOwnerAction(event.action),
      )
      .map((event) => Date.parse(event.at)),
  );
}
