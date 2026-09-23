/*
 * 「今天的时间线」（T-130 设计规范第 6 节第 4 条、第 7 节「规则同驾驶舱」）：驾驶舱与协作页共用的唯一实现（纯函数）。
 * 按执行者分泳道，黑块＝AI 工作，红块＝你的审批；全部由黑板汇总的 events 推出，不补任何数据。
 *
 * - 时间范围：汇总生成时刻向上取整到整点为终点，至少画最近 12 小时；今天更早就有可画事件时，
 *   从今天第一条所在的整点画起。
 * - AI 工作：只认执行体自己的活动事件（boardActions.ts 的 WORK_ACTIONS）；同一任务相邻两条事件之间算一段工作，
 *   相隔超过 {@link MAX_WORK_GAP_MS} 视为中间停过、断成两段；孤立的一条画成最窄块；同一执行者的段合并。
 *   AI 泳道最多 {@link MAX_AI_LANES} 条，按显示名排好。
 * - 你的审批：`execution.*` 是主人经执行接口做的决定（启动、取消、审阅、落地），intent 到 outcome 为一块；
 *   「你」这条泳道总在最后，空着也留着。
 * - 执行者一律取 executorGrouping：按根实例完整 id 分组，显示名按整份汇总算（overviewLabels）。
 */
import type { BoardOverview } from '../boardOverviewModel';
import type { Lane, LaneBlock, LaneTone } from '../ui';
import {
  TELEMETRY,
  WORK_ACTIONS,
  actionText,
  isOwnerAction,
} from './boardActions';
import {
  agentName,
  executorOf,
  overviewLabels,
  type ExecutorIdentity,
} from './executorGrouping';

const HOUR_MS = 3_600_000;
/** 时间线至少画最近这么久。 */
export const TIMELINE_SPAN_MS = 12 * HOUR_MS;
/** 同一任务相邻两条活动事件相隔超过这么久，就不算连续工作。 */
export const MAX_WORK_GAP_MS = 2 * HOUR_MS;
/** AI 泳道最多几条；多出的按最近活动取舍，并报出条数。 */
export const MAX_AI_LANES = 6;
/** 主人泳道的名字。 */
export const OWNER_LANE = '你';
/** 主人泳道的键；执行者的键都带 `instance:`／`adapter:` 前缀，不会与它相撞。 */
export const OWNER_KEY = 'owner';

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

/** 能画进泳道的事件：执行体自己的活动，或你的决定。 */
const drawable = (action: string) =>
  WORK_ACTIONS.has(action) || isOwnerAction(action);

/**
 * 汇总生成时刻所在的「今天」：黑板相关的时刻一律以服务端为准，不掺本机时钟；
 * 起点看今天最早一条能画进泳道的事件（执行体活动或你的决定）。
 */
export function overviewRange(overview: BoardOverview): TimelineRange {
  return timelineRange(
    Date.parse(overview.generatedAt),
    overview.events
      .filter((event) => drawable(event.action))
      .map((event) => Date.parse(event.at)),
  );
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

/** 一个任务的活动时刻切成工作段：按时间排好后，相邻两条相隔不超过上限就连成一段。 */
function workSegments(times: readonly number[]): Interval[] {
  const segments: Interval[] = [];
  for (const time of [...times].sort((a, b) => a - b)) {
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

const within = (ms: number, range: TimelineRange) =>
  ms >= range.from && ms <= range.to;

const byText = (left: string, right: string) =>
  left < right ? -1 : left > right ? 1 : 0;

/** 执行者显示名：取给定的名字表；表里没有（调用方给的表不全）时与拓扑同一个退路，只写类型。 */
function nameOf(
  labels: ReadonlyMap<string, string>,
  identity: ExecutorIdentity,
): string {
  return labels.get(identity.key) ?? agentName(identity.agentType);
}

/** 一条泳道：键是执行者分组键（executorGrouping 的 `instance:…`／`adapter:…`），「你」是 {@link OWNER_KEY}。 */
export interface TodayLane extends Lane {
  key: string;
}

export interface TodayLanesOptions {
  /** 画哪一段；缺省按 {@link overviewRange}。 */
  range?: TimelineRange;
  /**
   * 执行者显示名；缺省按这份汇总的全部条目算（overviewLabels）。只画汇总的一部分（比如按项目筛过）时，
   * 传整份汇总的名字表，同一执行者在各处才同名。
   */
  labels?: ReadonlyMap<string, string>;
}

export interface TodayLanes {
  range: TimelineRange;
  /** AI 泳道按显示名排好，最多 {@link MAX_AI_LANES} 条；「你」在最后，空着也留着。 */
  lanes: TodayLane[];
  /** 范围内有活动、但超出泳道上限没画出来的执行者数。 */
  hidden: number;
  /** 范围内能画进泳道的事件条数：执行体活动与你的决定（归属不到执行者的活动不算）。 */
  events: number;
}

/** 驾驶舱与协作页「今天的时间线」的唯一入口：同一份汇总，两处画出一模一样的泳道。 */
export function todayLanes(
  overview: BoardOverview,
  options: TodayLanesOptions = {},
): TodayLanes {
  const range = options.range ?? overviewRange(overview);
  const labels = options.labels ?? overviewLabels(overview);
  const byId = new Map(overview.items.map((item) => [item.id, item]));
  const work = new Map<string, number[]>();
  const owner: Interval[] = [];
  const openOwner = new Map<string, number>();
  let events = 0;
  for (const event of overview.events) {
    const ms = Date.parse(event.at);
    if (isOwnerAction(event.action)) {
      if (within(ms, range)) events += 1;
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
      if (byId.has(event.taskId) && within(ms, range)) events += 1;
      const times = work.get(event.taskId);
      if (times) times.push(ms);
      else work.set(event.taskId, [ms]);
    }
  }
  for (const start of openOwner.values()) owner.push({ start, end: start });

  // 执行者 → 工作段；汇总被截断时个别事件的任务可能不在 items 里，这些没法归属，不画。
  const byExecutor = new Map<
    string,
    { identity: ExecutorIdentity; intervals: Interval[] }
  >();
  for (const [taskId, times] of work) {
    const item = byId.get(taskId);
    if (!item) continue;
    const identity = executorOf(item);
    const entry = byExecutor.get(identity.key);
    if (entry) entry.intervals.push(...workSegments(times));
    else
      byExecutor.set(identity.key, {
        identity,
        intervals: workSegments(times),
      });
  }
  const active = [...byExecutor.values()]
    .map(({ identity, intervals }) => ({
      identity,
      intervals: merge(intervals).filter((interval) =>
        overlaps(interval, range),
      ),
    }))
    .filter((lane) => lane.intervals.length > 0);
  // 超过上限时留最近还有活动的几条，再按显示名排好。
  const lastEnd = (intervals: readonly Interval[]) =>
    Math.max(...intervals.map((interval) => interval.end));
  const kept = [...active]
    .sort((a, b) => lastEnd(b.intervals) - lastEnd(a.intervals))
    .slice(0, MAX_AI_LANES)
    .map((lane) => ({ ...lane, name: nameOf(labels, lane.identity) }))
    .sort(
      (a, b) =>
        byText(a.name, b.name) || byText(a.identity.key, b.identity.key),
    );

  const toBlocks = (
    intervals: readonly Interval[],
    tone: LaneTone,
  ): LaneBlock[] =>
    intervals.map((interval) => ({
      start: interval.start,
      end: interval.end,
      tone,
    }));
  return {
    range,
    lanes: [
      ...kept.map((lane) => ({
        key: lane.identity.key,
        name: lane.name,
        blocks: toBlocks(lane.intervals, 'work'),
      })),
      {
        key: OWNER_KEY,
        name: OWNER_LANE,
        blocks: toBlocks(
          merge(owner).filter((interval) => overlaps(interval, range)),
          'owner',
        ),
      },
    ],
    hidden: active.length - kept.length,
    events,
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
 * 最近的动态（驾驶舱时间线格用；协作页只画泳道）：时间范围内的 outcome 事件，去掉高频遥测，
 * 按时间倒序取前几条。执行者：`execution.*` 是你；其余是该任务的执行者显示名（与泳道同一套）。
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
          ? nameOf(labels, executorOf(item))
          : '—',
      text: `${actionText(event.action)} ${item?.title ?? event.taskId}`,
      raw: `${event.action}（${event.phase}）`,
    });
  }
  return entries;
}
